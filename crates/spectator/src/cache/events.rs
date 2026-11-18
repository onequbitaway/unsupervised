//! The recent public events.

use std::collections::VecDeque;
use std::sync::Arc;

use unsup_protocol::ResidentId;

use crate::wire::{EventId, FeedEvent, FeedPage};

/// A question to the event log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FeedQuery {
    /// Return events after this id. Without it: the newest events.
    pub after: Option<EventId>,
    /// Greatest number of events in the answer.
    pub limit: usize,
    /// Lowest drama score.
    pub min_drama: u8,
    /// The kind must start with this text.
    pub kind: Option<String>,
}

impl FeedQuery {
    fn lets_through(&self, event: &FeedEvent) -> bool {
        event.drama >= self.min_drama
            && self.kind.as_deref().is_none_or(|prefix| event.kind.starts_with(prefix))
    }
}

/// A bounded list of events in ascending id order.
#[derive(Debug)]
struct Ring {
    events: VecDeque<Arc<FeedEvent>>,
    capacity: usize,
    /// Id of the newest event that the ring dropped.
    dropped: Option<EventId>,
}

impl Ring {
    fn new(capacity: usize) -> Ring {
        Ring { events: VecDeque::new(), capacity, dropped: None }
    }

    fn push(&mut self, event: Arc<FeedEvent>) {
        if self.events.len() >= self.capacity
            && let Some(old) = self.events.pop_front()
        {
            self.dropped = Some(old.id);
        }
        if self.capacity > 0 {
            self.events.push_back(event);
        } else {
            self.dropped = Some(event.id);
        }
    }

    fn page(&self, query: &FeedQuery) -> FeedPage {
        let newest = self.events.back().map_or(EventId::START, |event| event.id);
        let Some(after) = query.after else {
            let mut events: Vec<FeedEvent> = self
                .events
                .iter()
                .rev()
                .filter(|event| query.lets_through(event))
                .take(query.limit)
                .map(|event| (**event).clone())
                .collect();
            events.reverse();
            return FeedPage { events, next: newest, more: false, gap: false };
        };
        if after > newest {
            // A cursor from another world. Start again at the newest event.
            return FeedPage { events: Vec::new(), next: newest, more: false, gap: false };
        }
        let start = self.events.partition_point(|event| event.id <= after);
        let mut page = FeedPage {
            events: Vec::new(),
            next: after,
            more: false,
            gap: self.dropped.is_some_and(|dropped| dropped > after),
        };
        for event in self.events.range(start..) {
            if query.lets_through(event) {
                if page.events.len() >= query.limit {
                    page.more = true;
                    break;
                }
                page.events.push((**event).clone());
            }
            page.next = event.id;
        }
        page
    }

    /// Returns the newest `limit` events about a resident, newest first.
    fn about(&self, resident: ResidentId, limit: usize) -> Vec<&Arc<FeedEvent>> {
        self.events.iter().rev().filter(|event| is_about(event, resident)).take(limit).collect()
    }
}

/// Returns true when the event is about the resident: the resident is the
/// actor, or the `resident` field of the data names it.
fn is_about(event: &FeedEvent, resident: ResidentId) -> bool {
    event.actor == Some(resident)
        || event.data.get("resident").and_then(serde_json::Value::as_u64) == Some(resident.0)
}

/// The public events that the cache keeps: every recent event, and the
/// notable events for a longer time.
#[derive(Debug)]
pub(crate) struct EventLog {
    recent: Ring,
    notable: Ring,
    notable_drama: u8,
}

impl EventLog {
    /// Makes an empty log.
    pub fn new(recent: usize, notable: usize, notable_drama: u8) -> EventLog {
        EventLog { recent: Ring::new(recent), notable: Ring::new(notable), notable_drama }
    }

    /// Adds an event. Ids must grow.
    pub fn push(&mut self, event: Arc<FeedEvent>) {
        if event.drama >= self.notable_drama {
            self.notable.push(Arc::clone(&event));
        }
        self.recent.push(event);
    }

    /// Answers a question. A question for notable events only reads the
    /// list that keeps them longer.
    pub fn page(&self, query: &FeedQuery) -> FeedPage {
        if query.min_drama >= self.notable_drama {
            self.notable.page(query)
        } else {
            self.recent.page(query)
        }
    }

    /// Returns the newest `limit` events about a resident, in ascending id
    /// order.
    pub fn about(&self, resident: ResidentId, limit: usize) -> Vec<FeedEvent> {
        let mut events = self.recent.about(resident, limit);
        events.extend(self.notable.about(resident, limit));
        events.sort_unstable_by_key(|event| event.id);
        events.dedup_by_key(|event| event.id);
        let skip = events.len().saturating_sub(limit);
        events.into_iter().skip(skip).map(|event| (**event).clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use unsup_protocol::{Clock, RealmId, Tick};

    use super::*;

    fn event(frame: u64, index: u32, kind: &str, drama: u8) -> Arc<FeedEvent> {
        Arc::new(FeedEvent {
            id: EventId { frame, index },
            frame,
            clock: Clock::default(),
            tick: Tick(frame),
            realm: RealmId::Global,
            zone: None,
            kind: kind.to_string(),
            actor: None,
            actor_name: None,
            pos: None,
            drama,
            data: Value::Null,
        })
    }

    fn ids(page: &FeedPage) -> Vec<String> {
        page.events.iter().map(|event| event.id.to_string()).collect()
    }

    fn query(after: Option<&str>, limit: usize) -> FeedQuery {
        FeedQuery {
            after: after.map(|text| text.parse().expect("an id")),
            limit,
            ..FeedQuery::default()
        }
    }

    fn log() -> EventLog {
        let mut log = EventLog::new(4, 3, 11);
        log.push(event(1, 0, "a.walked", 2));
        log.push(event(1, 1, "a.stole", 50));
        log.push(event(2, 0, "b.ate", 3));
        log.push(event(3, 0, "a.walked", 2));
        log.push(event(3, 1, "b.won", 90));
        log.push(event(4, 0, "a.walked", 2));
        log
    }

    #[test]
    fn without_a_cursor_the_page_holds_the_newest_events() {
        let page = log().page(&query(None, 3));
        assert_eq!(ids(&page), ["3-0", "3-1", "4-0"]);
        assert_eq!((page.next.to_string().as_str(), page.more, page.gap), ("4-0", false, false));
    }

    #[test]
    fn a_cursor_walks_forward_without_a_loss() {
        let log = log();
        let first = log.page(&query(Some("2-0"), 2));
        assert_eq!(ids(&first), ["3-0", "3-1"]);
        assert_eq!((first.next.to_string().as_str(), first.more, first.gap), ("3-1", true, false));
        let second = log.page(&query(Some("3-1"), 2));
        assert_eq!(ids(&second), ["4-0"]);
        assert_eq!((second.next.to_string().as_str(), second.more), ("4-0", false));
        let third = log.page(&query(Some("4-0"), 2));
        assert_eq!(ids(&third), Vec::<String>::new());
        assert_eq!(third.next.to_string(), "4-0");
    }

    #[test]
    fn a_cursor_older_than_the_log_reports_a_gap() {
        // The log keeps 4 events; `1-0` and `1-1` are gone.
        let page = log().page(&query(Some("1-0"), 10));
        assert_eq!(ids(&page), ["2-0", "3-0", "3-1", "4-0"]);
        assert!(page.gap);
        // `1-1` was the last event that the log dropped: nothing is lost after it.
        assert!(!log().page(&query(Some("1-1"), 10)).gap);
    }

    #[test]
    fn a_filter_moves_the_cursor_past_what_it_hides() {
        let log = log();
        let page = log.page(&FeedQuery { kind: Some("b.".into()), ..query(Some("0-0"), 1) });
        assert_eq!(ids(&page), ["2-0"]);
        // `3-0` does not match and is skipped; `3-1` matches and waits.
        assert_eq!((page.next.to_string().as_str(), page.more), ("3-0", true));
        let rest = log.page(&FeedQuery { kind: Some("b.".into()), ..query(Some("3-0"), 1) });
        assert_eq!(ids(&rest), ["3-1"]);
        assert_eq!((rest.next.to_string().as_str(), rest.more), ("4-0", false));
    }

    #[test]
    fn notable_events_stay_longer() {
        let log = log();
        // `1-1` left the recent list but the notable list still has it.
        let page = log.page(&FeedQuery { min_drama: 11, ..query(Some("0-0"), 10) });
        assert_eq!(ids(&page), ["1-1", "3-1"]);
        assert!(!page.gap);
        let page = log.page(&FeedQuery { min_drama: 60, ..query(None, 10) });
        assert_eq!(ids(&page), ["3-1"]);
    }

    #[test]
    fn a_cursor_from_another_world_starts_again() {
        let page = log().page(&query(Some("900-0"), 10));
        assert_eq!(ids(&page), Vec::<String>::new());
        assert_eq!((page.next.to_string().as_str(), page.gap), ("4-0", false));
        let empty = EventLog::new(4, 4, 11).page(&query(Some("900-0"), 10));
        assert_eq!(empty.next, EventId::START);
    }

    #[test]
    fn events_about_a_resident_come_from_both_lists_once() {
        let mut log = EventLog::new(2, 8, 11);
        let about = |frame: u64, drama: u8, actor: Option<u64>, data: Value| {
            let mut e = (*event(frame, 0, "x.y", drama)).clone();
            e.actor = actor.map(ResidentId);
            e.data = data;
            Arc::new(e)
        };
        log.push(about(1, 40, Some(7), Value::Null));
        log.push(about(2, 1, Some(8), Value::Null));
        log.push(about(3, 1, None, json!({ "resident": 7 })));
        log.push(about(4, 30, Some(7), Value::Null));
        log.push(about(5, 1, None, json!({ "resident": 8 })));
        let found: Vec<String> =
            log.about(ResidentId(7), 10).iter().map(|e| e.id.to_string()).collect();
        // `1-0` only in the notable list; `3-0` left the recent list (it
        // keeps 2); `4-0` is in both lists and comes once.
        assert_eq!(found, ["1-0", "4-0"]);
        let newest: Vec<String> =
            log.about(ResidentId(7), 1).iter().map(|e| e.id.to_string()).collect();
        assert_eq!(newest, ["4-0"]);
    }
}
