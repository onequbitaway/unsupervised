//! The chapters of a life story.

use serde::{Deserialize, Serialize};

use crate::config::ChaptersConfig;

/// A chapter of a life story.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Chapter {
    /// The resident came into the world.
    Born,
    /// The resident left the world.
    Death,
    /// Crimes, arrests and trials.
    Crimes,
    /// Public offices and powers.
    Offices,
    /// Jobs and businesses.
    Jobs,
    /// Homes and travel.
    Homes,
    /// Contracts, loans, marriages and organizations.
    Deals,
}

/// The starts of the event kinds of each chapter, in the order in which
/// an event is tried.
#[derive(Clone, Debug)]
pub(crate) struct Chapters {
    lists: Vec<(Chapter, Vec<String>)>,
}

impl Chapters {
    /// Reads the chapters of the configuration.
    pub fn new(config: &ChaptersConfig) -> Chapters {
        Chapters {
            lists: vec![
                (Chapter::Born, config.born.clone()),
                (Chapter::Death, config.death.clone()),
                (Chapter::Crimes, config.crimes.clone()),
                (Chapter::Offices, config.offices.clone()),
                (Chapter::Jobs, config.jobs.clone()),
                (Chapter::Homes, config.homes.clone()),
                (Chapter::Deals, config.deals.clone()),
            ],
        }
    }

    /// Returns the chapter of a kind of event: the first chapter with a
    /// start of the kind in its list.
    pub fn of(&self, kind: &str) -> Option<Chapter> {
        self.lists
            .iter()
            .find(|(_, starts)| starts.iter().any(|start| kind.starts_with(start.as_str())))
            .map(|(chapter, _)| *chapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn a_kind_goes_into_the_first_chapter_that_has_its_start() {
        let chapters = Chapters::new(&Config::default().chapters);
        assert_eq!(chapters.of("kernel.spawned"), Some(Chapter::Born));
        assert_eq!(chapters.of("life.resident_died"), Some(Chapter::Death));
        assert_eq!(chapters.of("kernel.despawned"), Some(Chapter::Death));
        assert_eq!(chapters.of("crime.theft_committed"), Some(Chapter::Crimes));
        assert_eq!(chapters.of("politics.election_decided"), Some(Chapter::Offices));
        assert_eq!(chapters.of("economy.worker_hired"), Some(Chapter::Jobs));
        assert_eq!(chapters.of("economy.lease_started"), Some(Chapter::Homes));
        assert_eq!(chapters.of("society.contract_signed"), Some(Chapter::Deals));
        assert_eq!(chapters.of("society.married"), Some(Chapter::Deals));
        assert_eq!(chapters.of("society.said"), None);
        assert_eq!(chapters.of("economy.shift_completed"), None);
    }
}
