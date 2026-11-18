//! The sizes of a mission that the route checks before the mission goes
//! to the simulation.
//!
//! The limits are parameters of the missions module. The route reads
//! their values from the catalogue of the world when the server starts,
//! so that a mission that is too large gets its answer at the door: it
//! does not get into the input log, and the module does not have to
//! refuse it in public. The module checks every mission again with the
//! values of that moment, and it checks what only it knows: the kinds of
//! events, the ranges of the numbers and the money of the day.

use unsup_missions::MissionSpec;
use unsup_protocol::Catalogue;

/// The sizes that the missions module permits, in bytes and in counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    /// Bytes of a title.
    title: usize,
    /// Bytes of a text.
    text: usize,
    /// Objectives of one mission.
    objectives: usize,
    /// Data fields that one objective compares.
    fields: usize,
    /// Bytes of an event kind.
    event: usize,
    /// Bytes of the name of a data field.
    field_name: usize,
    /// Bytes of a text value of a data field.
    field_text: usize,
    /// Bytes of the name of a sponsor.
    sponsor: usize,
}

impl Sizes {
    /// Reads the sizes from the parameters of the missions module.
    ///
    /// # Errors
    ///
    /// The name of the first parameter that the catalogue does not have:
    /// the world has no missions module, or the module has other names.
    pub(crate) fn read(catalogue: &Catalogue) -> Result<Sizes, String> {
        let size = |name: &str| {
            let param = catalogue.params.iter().find(|param| param.name == name);
            let value = param.ok_or_else(|| name.to_string())?.value;
            Ok::<usize, String>(usize::try_from(value).unwrap_or(0))
        };
        Ok(Sizes {
            title: size("missions.title_max")?,
            text: size("missions.text_max")?,
            objectives: size("missions.objectives_max")?,
            fields: size("missions.match_max")?,
            event: size("missions.event_kind_max")?,
            field_name: size("missions.field_name_max")?,
            field_text: size("missions.field_text_max")?,
            sponsor: size("missions.sponsor_name_max")?,
        })
    }

    /// Checks the sizes of a mission and of the name of its sponsor.
    ///
    /// # Errors
    ///
    /// One sentence for the sponsor that names the field and the limit.
    pub(crate) fn check(&self, spec: &MissionSpec, sponsor: &str) -> Result<(), String> {
        if sponsor.len() > self.sponsor {
            return Err(format!(
                "The name of the owner has more than {} bytes. A sponsor needs a shorter name.",
                self.sponsor
            ));
        }
        if spec.title.trim().is_empty() {
            return Err("`title` must not be empty.".into());
        }
        if spec.title.len() > self.title {
            return Err(format!("`title` has at most {} bytes.", self.title));
        }
        if spec.text.len() > self.text {
            return Err(format!("`text` has at most {} bytes.", self.text));
        }
        if spec.objectives.is_empty() || spec.objectives.len() > self.objectives {
            return Err(format!("`objectives` has from 1 to {} objectives.", self.objectives));
        }
        for (index, objective) in spec.objectives.iter().enumerate() {
            let place = format!("`objectives[{index}]`");
            if objective.event.len() > self.event {
                return Err(format!("{place}: `event` has at most {} bytes.", self.event));
            }
            if objective.fields.len() > self.fields {
                return Err(format!("{place}: `fields` has at most {} data fields.", self.fields));
            }
            if objective.fields.keys().any(|name| name.len() > self.field_name) {
                return Err(format!(
                    "{place}: the name of a data field has at most {} bytes.",
                    self.field_name
                ));
            }
            let long = |value: &serde_json::Value| {
                value.as_str().is_some_and(|text| text.len() > self.field_text)
            };
            if objective.fields.values().any(long) {
                return Err(format!(
                    "{place}: a text value of a data field has at most {} bytes.",
                    self.field_text
                ));
            }
        }
        Ok(())
    }
}
