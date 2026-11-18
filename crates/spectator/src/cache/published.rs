//! Published state: the newest event of each kind that ends in `_published`.

use std::collections::BTreeMap;

use serde_json::Value;
use unsup_protocol::{Clock, Event};

use crate::wire::{Published, PublishedRef};

/// The end of the kind of an event that carries public state.
pub(crate) const PUBLISHED_SUFFIX: &str = "_published";

/// The newest event of each kind and key.
#[derive(Debug)]
pub(crate) struct PublishedStore {
    series: BTreeMap<(String, Option<String>), Published>,
    max_series: usize,
    max_key_bytes: usize,
}

/// Why an event was not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The `key` field is not text, a number or a boolean.
    KeyType,
    /// The key is longer than the limit.
    KeyLength,
}

/// Returns the key of a published event as text. The key is the `key`
/// field of the data: text, a number or a boolean.
fn key_of(data: &Value) -> Result<Option<String>, Refused> {
    match data.get("key") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(Value::Bool(flag)) => Ok(Some(flag.to_string())),
        Some(Value::Array(_) | Value::Object(_)) => Err(Refused::KeyType),
    }
}

impl PublishedStore {
    /// Makes an empty store.
    pub fn new(max_series: usize, max_key_bytes: usize) -> PublishedStore {
        PublishedStore { series: BTreeMap::new(), max_series, max_key_bytes }
    }

    /// Keeps an event as the newest of its kind and key. When the store is
    /// full, a new series replaces the series that is oldest.
    pub fn put(&mut self, event: &Event, frame: u64, clock: Clock) -> Result<(), Refused> {
        let key = key_of(&event.data)?;
        if key.as_ref().is_some_and(|key| key.len() > self.max_key_bytes) {
            return Err(Refused::KeyLength);
        }
        let slot = (event.kind.clone(), key.clone());
        if !self.series.contains_key(&slot) && self.series.len() >= self.max_series {
            let oldest = self
                .series
                .iter()
                .min_by_key(|(_, published)| published.frame)
                .map(|(slot, _)| slot.clone());
            if let Some(oldest) = oldest {
                self.series.remove(&oldest);
            }
        }
        self.series.insert(
            slot,
            Published { kind: event.kind.clone(), key, frame, clock, data: event.data.clone() },
        );
        Ok(())
    }

    /// Returns one series. `None` as the key is the series without a key.
    pub fn get(&self, kind: &str, key: Option<&str>) -> Option<&Published> {
        self.series.get(&(kind.to_string(), key.map(str::to_string)))
    }

    /// Returns every series of a kind, in ascending key order.
    pub fn of_kind(&self, kind: &str) -> Vec<Published> {
        self.series
            .range((kind.to_string(), None)..)
            .take_while(|((k, _), _)| k == kind)
            .map(|(_, published)| published.clone())
            .collect()
    }

    /// Returns one line for every series.
    pub fn index(&self) -> Vec<PublishedRef> {
        self.series
            .values()
            .map(|p| PublishedRef { kind: p.kind.clone(), key: p.key.clone(), frame: p.frame })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use unsup_protocol::{RealmId, Tick};

    use super::*;

    fn event(kind: &str, data: Value) -> Event {
        Event {
            seq: 1,
            frame: 1,
            tick: Tick(1),
            realm: RealmId::Global,
            kind: kind.to_string(),
            actor: None,
            pos: None,
            public: true,
            drama: 0,
            data,
        }
    }

    fn keys(store: &PublishedStore) -> Vec<(String, Option<String>, u64)> {
        store.index().into_iter().map(|r| (r.kind, r.key, r.frame)).collect()
    }

    #[test]
    fn the_newest_event_of_each_kind_and_key_stays() {
        let mut store = PublishedStore::new(8, 16);
        let clock = Clock::default();
        store.put(&event("f.stats_published", json!({ "m": 1 })), 1, clock).expect("kept");
        store
            .put(&event("m.board_published", json!({ "key": "rich", "n": 1 })), 2, clock)
            .expect("kept");
        store
            .put(&event("m.board_published", json!({ "key": 3, "n": 2 })), 3, clock)
            .expect("kept");
        store
            .put(&event("m.board_published", json!({ "key": "rich", "n": 4 })), 4, clock)
            .expect("kept");
        assert_eq!(
            keys(&store),
            [
                ("f.stats_published".to_string(), None, 1),
                ("m.board_published".to_string(), Some("3".to_string()), 3),
                ("m.board_published".to_string(), Some("rich".to_string()), 4),
            ]
        );
        assert_eq!(store.get("m.board_published", Some("rich")).expect("rich").data["n"], 4);
        assert_eq!(store.get("f.stats_published", None).expect("stats").data, json!({ "m": 1 }));
        assert_eq!(store.get("f.stats_published", Some("x")), None);
        assert_eq!(store.of_kind("m.board_published").len(), 2);
        assert_eq!(store.of_kind("m.board").len(), 0);
    }

    #[test]
    fn a_bad_key_is_refused() {
        let mut store = PublishedStore::new(8, 4);
        let clock = Clock::default();
        assert_eq!(
            store.put(&event("a_published", json!({ "key": [1] })), 1, clock),
            Err(Refused::KeyType)
        );
        assert_eq!(
            store.put(&event("a_published", json!({ "key": "12345" })), 1, clock),
            Err(Refused::KeyLength)
        );
        assert_eq!(store.put(&event("a_published", json!({ "key": true })), 1, clock), Ok(()));
        assert_eq!(keys(&store), [("a_published".to_string(), Some("true".to_string()), 1)]);
    }

    #[test]
    fn a_full_store_drops_the_oldest_series() {
        let mut store = PublishedStore::new(2, 16);
        let clock = Clock::default();
        store.put(&event("a_published", json!({ "key": "x" })), 5, clock).expect("kept");
        store.put(&event("a_published", json!({ "key": "y" })), 3, clock).expect("kept");
        // A new value of a series that is there needs no room.
        store.put(&event("a_published", json!({ "key": "y" })), 6, clock).expect("kept");
        store.put(&event("a_published", json!({ "key": "z" })), 7, clock).expect("kept");
        assert_eq!(
            keys(&store),
            [
                ("a_published".to_string(), Some("y".to_string()), 6),
                ("a_published".to_string(), Some("z".to_string()), 7),
            ]
        );
    }
}
