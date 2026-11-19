//! Who and what an event is about.

use serde_json::Value;
use unsup_protocol::{CityId, ResidentId, SiteId};

use crate::config::FactsConfig;
use crate::wire::FeedEvent;

/// Who and what an event is about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The residents of the event: the actor first, then the residents
    /// that the data names, each one time.
    pub residents: Vec<ResidentId>,
    /// The site that the data names.
    pub site: Option<SiteId>,
    /// The city that the data names.
    pub city: Option<CityId>,
}

/// Reads the residents that a data value names: a resident id, a holder
/// that is a resident, or a list of those.
pub(crate) fn residents_in(value: &Value, out: &mut Vec<ResidentId>) {
    match value {
        Value::Number(number) => {
            if let Some(id) = number.as_u64() {
                out.push(ResidentId(id));
            }
        }
        Value::Array(list) => {
            for entry in list {
                residents_in(entry, out);
            }
        }
        // The JSON form of `Holder::Resident`.
        Value::Object(map) if map.len() == 1 => {
            if let Some(id) =
                map.get("Resident").or_else(|| map.get("resident")).and_then(Value::as_u64)
            {
                out.push(ResidentId(id));
            }
        }
        _ => {}
    }
}

/// Reads events by the field names of the configuration.
#[derive(Clone, Debug)]
pub(crate) struct Reader {
    config: FactsConfig,
}

impl Reader {
    /// Makes a reader.
    pub fn new(config: &FactsConfig) -> Reader {
        Reader { config: config.clone() }
    }

    /// Returns who and what an event is about.
    pub fn facts(&self, event: &FeedEvent) -> Facts {
        let mut named = Vec::new();
        named.extend(event.actor);
        for field in &self.config.resident_fields {
            if let Some(value) = event.data.get(field) {
                residents_in(value, &mut named);
            }
        }
        let mut residents: Vec<ResidentId> = Vec::new();
        for id in named {
            if residents.len() >= self.config.max_residents {
                break;
            }
            if !residents.contains(&id) {
                residents.push(id);
            }
        }
        let number = |fields: &[String]| {
            fields.iter().find_map(|field| event.data.get(field).and_then(Value::as_u64))
        };
        Facts {
            residents,
            site: number(&self.config.site_fields).map(SiteId),
            city: number(&self.config.city_fields)
                .and_then(|city| u16::try_from(city).ok())
                .map(CityId),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Config;
    use crate::facts::test_event;

    #[test]
    fn the_actor_comes_first_and_each_resident_one_time() {
        let reader = Reader::new(&Config::default().facts);
        let event = test_event(
            1,
            0,
            "crime.arrest_made",
            55,
            Some(2),
            json!({ "officer": 2, "target": 1, "case": 7, "cases": [7], "site": 900, "city": 1 }),
        );
        assert_eq!(
            reader.facts(&event),
            Facts {
                residents: vec![ResidentId(2), ResidentId(1)],
                site: Some(SiteId(900)),
                city: Some(CityId(1)),
            }
        );
    }

    #[test]
    fn a_count_is_not_read_as_a_resident() {
        // The politics module counts the candidates of an election and the
        // members of a vote, and the crime module the jurors of a trial.
        let reader = Reader::new(&Config::default().facts);
        let event = test_event(
            1,
            0,
            "politics.candidate_registered",
            20,
            Some(4),
            json!({ "candidate": 4, "candidates": 2, "members": 3, "jurors": 1, "second": 1 }),
        );
        assert_eq!(reader.facts(&event).residents, vec![ResidentId(4)]);
    }

    #[test]
    fn holders_and_lists_name_residents() {
        let reader = Reader::new(&Config::default().facts);
        let event = test_event(
            1,
            0,
            "economy.property_sold",
            30,
            None,
            json!({
                "from": { "Org": 100 }, "to": { "Resident": 7 }, "parties": [3, { "resident": 4 }],
                "price": 9, "site": "not a number",
            }),
        );
        let facts = reader.facts(&event);
        // The fields are read in the order of the configuration: `to`
        // comes before `parties`. An organization is not a resident.
        assert_eq!(facts.residents, vec![ResidentId(7), ResidentId(3), ResidentId(4)]);
        assert_eq!((facts.site, facts.city), (None, None));
    }

    #[test]
    fn an_event_has_a_bounded_number_of_residents() {
        let mut config = Config::default().facts;
        config.max_residents = 3;
        let reader = Reader::new(&config);
        let event = test_event(1, 0, "x.y", 30, Some(9), json!({ "parties": [1, 2, 3, 4, 5] }));
        assert_eq!(
            reader.facts(&event).residents,
            vec![ResidentId(9), ResidentId(1), ResidentId(2)]
        );
    }
}
