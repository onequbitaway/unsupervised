//! The words for an event: one line from a template of the data file.
//!
//! A template is text with slots. A slot is `{what:field|default}`. The
//! field and the default are optional for some slots. See
//! `data/director.toml` for the list of slots.

use std::collections::BTreeMap;

use serde_json::Value;
use unsup_protocol::{CityId, ResidentId, SiteId};

use super::Atlas;
use super::subjects::residents_in;
use super::text::{clean, humanize, money};
use crate::config::{FactsConfig, Phrases};
use crate::wire::FeedEvent;

/// What a slot shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// The name of the actor.
    Actor,
    /// The name of the residents in a data field.
    Name,
    /// Cents as money.
    Money,
    /// A short text or a number.
    Word,
    /// Text that a resident wrote.
    Text,
    /// The name of the site in a data field.
    Site,
    /// The name of the city in a data field.
    City,
    /// The site, the district or the city of the event.
    Place,
}

/// One part of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    /// Text as it is.
    Text(String),
    /// A slot with its field and its default.
    Slot { slot: Slot, field: String, default: Option<String> },
}

/// A line with slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Template {
    parts: Vec<Part>,
}

impl Template {
    /// Reads a template.
    ///
    /// # Errors
    ///
    /// Fails with a sentence for the operator when a brace does not close,
    /// when a slot has an unknown name, or when a slot lacks its field.
    pub fn parse(text: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_string()));
            }
            let after = &rest[open + 1..];
            let close =
                after.find('}').ok_or_else(|| format!("`{text}` has a `{{` without a `}}`"))?;
            parts.push(Template::slot(&after[..close], text)?);
            rest = &after[close + 1..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_string()));
        }
        Ok(Template { parts })
    }

    fn slot(inner: &str, text: &str) -> Result<Part, String> {
        let (head, default) = match inner.split_once('|') {
            Some((head, default)) => (head, Some(default.to_string())),
            None => (inner, None),
        };
        let (name, field) = head.split_once(':').unwrap_or((head, ""));
        let slot = match name {
            "actor" => Slot::Actor,
            "name" => Slot::Name,
            "money" => Slot::Money,
            "word" => Slot::Word,
            "text" => Slot::Text,
            "site" => Slot::Site,
            "city" => Slot::City,
            "place" => Slot::Place,
            other => return Err(format!("`{text}` has the unknown slot `{other}`")),
        };
        let needs_field = !matches!(slot, Slot::Actor | Slot::Place);
        if needs_field == field.is_empty() {
            return Err(if needs_field {
                format!("the slot `{name}` of `{text}` needs a field: `{{{name}:field}}`")
            } else {
                format!("the slot `{name}` of `{text}` takes no field")
            });
        }
        Ok(Part::Slot { slot, field: field.to_string(), default })
    }

    /// Writes the line of an event. The result is one line of plain text.
    pub fn render(&self, event: &FeedEvent, atlas: &dyn Atlas, limits: &FactsConfig) -> String {
        let mut line = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => line.push_str(text),
                Part::Slot { slot, field, default } => {
                    let value = fill(*slot, field, event, atlas, limits)
                        .filter(|value| !value.is_empty())
                        .or_else(|| default.clone())
                        .unwrap_or_else(|| fallback(*slot).to_string());
                    line.push_str(&value);
                }
            }
        }
        clean(&line, limits.line_chars)
    }
}

/// The words of a slot that has no value and no default.
fn fallback(slot: Slot) -> &'static str {
    match slot {
        Slot::Actor => "Someone",
        Slot::Name => "someone",
        Slot::Money => "some money",
        Slot::Word | Slot::Text => "",
        Slot::Site => "a building",
        Slot::City => "a city",
        Slot::Place => "somewhere",
    }
}

/// Returns the name of a resident for a line.
pub(crate) fn resident_name(id: ResidentId, atlas: &dyn Atlas, limits: &FactsConfig) -> String {
    atlas
        .resident_name(id)
        .map(|name| clean(&name, limits.name_chars))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("resident {id}"))
}

/// Returns the value of a slot, or `None` when the event does not have it.
fn fill(
    slot: Slot,
    field: &str,
    event: &FeedEvent,
    atlas: &dyn Atlas,
    limits: &FactsConfig,
) -> Option<String> {
    let value = || event.data.get(field);
    match slot {
        Slot::Actor => {
            let given = event.actor_name.as_deref().map(|name| clean(name, limits.name_chars));
            given
                .filter(|name| !name.is_empty())
                .or_else(|| event.actor.map(|id| resident_name(id, atlas, limits)))
        }
        Slot::Name => {
            let mut ids = Vec::new();
            residents_in(value()?, &mut ids);
            ids.dedup();
            if ids.is_empty() {
                return None;
            }
            let names: Vec<String> = ids
                .iter()
                .take(limits.slot_names)
                .map(|id| {
                    // The event carries the name of its actor.
                    match (&event.actor_name, event.actor == Some(*id)) {
                        (Some(name), true) if !clean(name, limits.name_chars).is_empty() => {
                            clean(name, limits.name_chars)
                        }
                        _ => resident_name(*id, atlas, limits),
                    }
                })
                .collect();
            let mut text = names.join(" and ");
            if ids.len() > limits.slot_names {
                text.push_str(" and others");
            }
            Some(text)
        }
        Slot::Money => value()?.as_i64().map(money),
        Slot::Word => match value()? {
            Value::String(text) => Some(clean(&text.replace('_', " "), limits.word_chars)),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        },
        Slot::Text => {
            let value = value()?;
            let text = match value {
                Value::String(text) => Some(text.as_str()),
                // The JSON form of `Untrusted`.
                Value::Object(map) => map.get("text").and_then(Value::as_str),
                _ => None,
            };
            let limit = if field == "name" { limits.name_chars } else { limits.text_chars };
            text.map(|text| clean(text, limit))
        }
        Slot::Site => {
            let site = atlas.site(SiteId(value()?.as_u64()?))?;
            Some(clean(&site.name, limits.name_chars))
        }
        Slot::City => atlas.city_name(CityId(u16::try_from(value()?.as_u64()?).ok()?)),
        Slot::Place => {
            let zone = event.zone?;
            let site = event.pos.and_then(|pos| atlas.site_at(zone, pos));
            site.map(|site| clean(&site.name, limits.name_chars))
                .filter(|name| !name.is_empty())
                .or_else(|| atlas.zone(zone).map(|(name, _)| name))
        }
    }
}

/// The words for every kind of event.
#[derive(Clone, Debug)]
pub(crate) struct Phrasebook {
    templates: BTreeMap<String, Template>,
    limits: FactsConfig,
}

impl Phrasebook {
    /// Reads the templates of the configuration.
    ///
    /// # Errors
    ///
    /// Fails with a sentence for the operator when a template does not
    /// parse.
    pub fn new(phrases: &Phrases, limits: &FactsConfig) -> Result<Phrasebook, String> {
        let mut templates = BTreeMap::new();
        for (kind, text) in phrases {
            templates.insert(kind.clone(), Template::parse(text)?);
        }
        Ok(Phrasebook { templates, limits: limits.clone() })
    }

    /// Makes a phrasebook without templates: every kind gets its name as
    /// words.
    pub fn plain(limits: &FactsConfig) -> Phrasebook {
        Phrasebook { templates: BTreeMap::new(), limits: limits.clone() }
    }

    /// Writes one line for an event. A kind without a template gets its
    /// name as words, with the name of the actor.
    pub fn line(&self, event: &FeedEvent, atlas: &dyn Atlas) -> String {
        if let Some(template) = self.templates.get(&event.kind) {
            return template.render(event, atlas, &self.limits);
        }
        let what = humanize(&event.kind, self.limits.line_chars);
        let line = match fill(Slot::Actor, "", event, atlas, &self.limits) {
            Some(actor) if !actor.is_empty() => format!("{what}: {actor}"),
            _ => what,
        };
        clean(&line, self.limits.line_chars)
    }

    /// Returns the name of a resident for a line.
    pub fn resident(&self, id: ResidentId, atlas: &dyn Atlas) -> String {
        resident_name(id, atlas, &self.limits)
    }

    /// Returns the limits of the lines.
    pub fn limits(&self) -> &FactsConfig {
        &self.limits
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Config;
    use crate::facts::{TestAtlas, test_event};

    fn book() -> Phrasebook {
        let config = Config::default();
        Phrasebook::new(&config.phrases, &config.facts).expect("the built-in phrases parse")
    }

    #[test]
    fn a_line_names_who_did_what_to_whom() {
        let atlas = TestAtlas::town();
        // The data of the crime module: an arrest by an officer.
        let arrest = test_event(
            3,
            0,
            "crime.arrest_made",
            55,
            Some(2),
            json!({ "officer": 2, "target": 1, "case": 7, "cases": [7], "city": 0, "watch": false }),
        );
        assert_eq!(book().line(&arrest, &atlas), "Bo arrested Ada");
        let married = test_event(3, 1, "society.married", 30, Some(3), json!({ "a": 3, "b": 4 }));
        assert_eq!(book().line(&married, &atlas), "Cleo and Dev married");
        let signed =
            test_event(3, 2, "society.contract_signed", 20, None, json!({ "parties": [1, 77] }));
        assert_eq!(book().line(&signed, &atlas), "Ada and resident 77 signed a contract");
    }

    #[test]
    fn a_slot_without_a_value_uses_its_default() {
        let atlas = TestAtlas::town();
        // An arrest by the city watch has no officer.
        let arrest = test_event(3, 0, "crime.arrest_made", 55, None, json!({ "officer": null }));
        assert_eq!(book().line(&arrest, &atlas), "The city watch arrested a suspect");
        let opened = test_event(
            3,
            0,
            "economy.business_opened",
            25,
            Some(1),
            json!({ "owner": { "Resident": 1 }, "site": 900, "kind": "coffee_shop" }),
        );
        assert_eq!(book().line(&opened, &atlas), "Ada opened a coffee shop at First Bank");
        let unknown_site = test_event(
            3,
            0,
            "economy.business_opened",
            25,
            Some(1),
            json!({ "owner": { "Org": 5 }, "site": 1 }),
        );
        assert_eq!(book().line(&unknown_site, &atlas), "Someone opened a business at a new place");
    }

    #[test]
    fn places_and_cities_have_names() {
        let atlas = TestAtlas::town();
        let spawned = test_event(1, 0, "kernel.spawned", 10, Some(1), json!({}));
        // The event is at (5, 5), where no site is: the district names the place.
        assert_eq!(book().line(&spawned, &atlas), "Ada arrived in Old Town");
        let election = test_event(
            9,
            0,
            "politics.election_decided",
            85,
            Some(4),
            json!({ "gov": "city", "city": 1, "office": "mayor", "winner": 4, "winners": [4] }),
        );
        assert_eq!(book().line(&election, &atlas), "Dev won the election for mayor in Tokentown");
        // A crime has no actor. The place of the event names where it was.
        let mut theft = (*test_event(
            9,
            1,
            "crime.theft_committed",
            45,
            None,
            json!({ "case": 7, "kind": "theft", "city": 0, "site": null, "value": 4500, "attempt": false }),
        ))
        .clone();
        assert_eq!(book().line(&theft, &atlas), "A theft took $45.00 in Old Town");
        theft.zone = None;
        assert_eq!(book().line(&theft, &atlas), "A theft took $45.00 in the city");
    }

    #[test]
    fn text_of_a_resident_is_one_plain_bounded_line() {
        let atlas = TestAtlas::town();
        let said = test_event(
            3,
            0,
            "society.said",
            3,
            Some(1),
            json!({ "text": { "untrusted": true, "from": 1, "text": "line one\nline\u{202e} two" } }),
        );
        assert_eq!(book().line(&said, &atlas), "Ada: “line one line two”");
        let mut long = (*said).clone();
        long.data = json!({ "text": "x".repeat(500) });
        let line = book().line(&long, &atlas);
        assert_eq!(line.chars().count(), "Ada: “”".chars().count() + 80);
        assert!(line.ends_with("…”"), "{line}");
        let mut named = (*said).clone();
        named.actor_name = Some(format!("{}\n<b>", "N".repeat(100)));
        let line = book().line(&named, &atlas);
        assert!(line.starts_with(&format!("{}…: “", "N".repeat(39))), "{line}");
    }

    #[test]
    fn a_kind_without_words_gets_its_name() {
        let atlas = TestAtlas::town();
        let event = test_event(3, 0, "crime.heist_planned", 60, Some(3), json!({}));
        assert_eq!(book().line(&event, &atlas), "Heist planned: Cleo");
        let nobody = test_event(3, 0, "politics.law_passed", 60, None, json!({}));
        assert_eq!(book().line(&nobody, &atlas), "Law passed");
    }

    #[test]
    fn a_bad_template_is_refused() {
        assert_eq!(
            Template::parse("{actor} did {what"),
            Err("`{actor} did {what` has a `{` without a `}`".to_string())
        );
        assert_eq!(
            Template::parse("{hero} won"),
            Err("`{hero} won` has the unknown slot `hero`".to_string())
        );
        assert_eq!(
            Template::parse("{money} was paid"),
            Err("the slot `money` of `{money} was paid` needs a field: `{money:field}`".to_string())
        );
        assert_eq!(
            Template::parse("{place:site}"),
            Err("the slot `place` of `{place:site}` takes no field".to_string())
        );
    }
}
