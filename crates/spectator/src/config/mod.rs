//! The configuration of the spectator service.
//!
//! The numbers live in two data files. `data/spectator.toml` has the
//! limits: sizes, counts and times. `data/director.toml` has the editorial
//! data: the weights of the director, the rules of the stories, the
//! chapters of a life story and the words for events. This module only
//! reads them.

mod editorial;
mod limits;
mod validate;

use std::fmt;

use serde::Deserialize;

pub use self::editorial::{ChaptersConfig, DirectorConfig, FactsConfig, Phrases, StoryKind};
pub use self::limits::{
    ChronicleConfig, ClipsConfig, FeedConfig, MapConfig, ProfileConfig, PublishedConfig,
    SocketConfig, StartConfig, StoriesConfig, StreamConfig,
};

/// The built-in configuration: the two data files in a row.
const BUILT_IN: &str = concat!(
    include_str!("../../data/spectator.toml"),
    "\n",
    include_str!("../../data/director.toml")
);

/// The configuration of the spectator service. [`Config::default`] gives
/// the values of `data/spectator.toml` and `data/director.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Limits of the event feed.
    pub feed: FeedConfig,
    /// Limits of one viewer connection.
    pub socket: SocketConfig,
    /// Limits of resident profiles.
    pub profile: ProfileConfig,
    /// Limits of the zone maps.
    pub map: MapConfig,
    /// How the cache starts.
    pub start: StartConfig,
    /// Limits of the published state.
    pub published: PublishedConfig,
    /// The delay of everything that viewers get.
    pub stream: StreamConfig,
    /// Limits of the stories.
    pub stories: StoriesConfig,
    /// Limits of the chronicle.
    pub chronicle: ChronicleConfig,
    /// Limits of the replay clips.
    pub clips: ClipsConfig,
    /// How the director scores events and chooses shots.
    pub director: DirectorConfig,
    /// How the service reads the data of an event.
    pub facts: FactsConfig,
    /// The rules of the stories, one for each kind.
    #[serde(rename = "story")]
    pub story_kinds: Vec<StoryKind>,
    /// The chapters of a life story.
    pub chapters: ChaptersConfig,
    /// The words for each kind of event.
    pub phrases: Phrases,
}

/// A configuration that the service cannot run with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid spectator configuration: {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads a configuration from TOML text and checks it. The text has
    /// every table of `data/spectator.toml` and of `data/director.toml`.
    ///
    /// # Errors
    ///
    /// Fails when the text does not parse, when a field is absent or
    /// unknown, or when [`Config::validate`] fails.
    pub fn from_toml(text: &str) -> Result<Config, ConfigError> {
        let config: Config =
            toml::from_str(text).map_err(|error| ConfigError { message: error.to_string() })?;
        config.validate()?;
        Ok(config)
    }

    /// Returns the text of the built-in configuration: the two data files
    /// in a row.
    #[must_use]
    pub fn built_in_toml() -> &'static str {
        BUILT_IN
    }

    /// Reads a configuration that changes some values of the built-in one,
    /// and checks it. The text has only the tables and the values to
    /// change, for example `[stream]` with `delay_frames = 300`. A list
    /// (the `[[story]]` rules, a list of prefixes) replaces the built-in
    /// list as a whole.
    ///
    /// # Errors
    ///
    /// Fails when the text does not parse, when it has an unknown table or
    /// field, or when [`Config::validate`] fails.
    pub fn with_overrides(text: &str) -> Result<Config, ConfigError> {
        let parse = |text: &str| {
            text.parse::<toml::Table>().map_err(|error| ConfigError { message: error.to_string() })
        };
        let mut merged = parse(BUILT_IN)?;
        merge(&mut merged, parse(text)?);
        let config: Config = merged
            .try_into()
            .map_err(|error: toml::de::Error| ConfigError { message: error.to_string() })?;
        config.validate()?;
        Ok(config)
    }
}

/// Puts the values of `over` into `base`. A table goes into the table of
/// the same name; every other value replaces the value of the same name.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(inner)), toml::Value::Table(over)) => merge(inner, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

impl Default for Config {
    fn default() -> Config {
        // The text is two files of this crate, read at compile time. The
        // test `the_built_in_configuration_is_valid` proves that it parses
        // and is valid.
        Config::from_toml(BUILT_IN).expect("the data files of the crate are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_configuration_is_valid() {
        let config = Config::from_toml(BUILT_IN).expect("the built-in configuration parses");
        assert_eq!(config, Config::default());
        assert_eq!(config.feed.headline_drama, 71);
        assert_eq!(config.published.stats_kind, "finance.stats_published");
        assert_eq!(config.stream.delay_frames, 0);
        assert_eq!(config.director.breaking_drama, 71);
        assert_eq!(config.story_kinds[0].kind, "crime");
    }

    #[test]
    fn unknown_and_absent_fields_are_refused() {
        let unknown = BUILT_IN.replace("[map]", "[map]\nsurprise = 1");
        let error = Config::from_toml(&unknown).expect_err("an unknown field");
        assert!(error.message.contains("surprise"), "{error}");

        let absent = BUILT_IN.replace("refresh_frames = 60", "");
        let error = Config::from_toml(&absent).expect_err("an absent field");
        assert!(error.message.contains("refresh_frames"), "{error}");

        let absent = BUILT_IN.replace("breaking_drama = 71", "");
        let error = Config::from_toml(&absent).expect_err("an absent weight");
        assert!(error.message.contains("breaking_drama"), "{error}");
    }

    #[test]
    fn overrides_change_only_what_they_name() {
        assert_eq!(Config::with_overrides("").expect("no change"), Config::default());
        let config = Config::with_overrides(
            "[stream]\ndelay_frames = 300\n[director]\nmin_hold_frames = 20\n\
             [phrases]\n\"town.fair_opened\" = \"{actor} opened the fair\"\n\
             [chapters]\nborn = [\"kernel.spawned\", \"life.born\"]\n",
        )
        .expect("valid overrides");
        let mut expected = Config::default();
        expected.stream.delay_frames = 300;
        expected.director.min_hold_frames = 20;
        expected.phrases.insert("town.fair_opened".into(), "{actor} opened the fair".into());
        expected.chapters.born = vec!["kernel.spawned".into(), "life.born".into()];
        assert_eq!(config, expected);
    }

    #[test]
    fn bad_overrides_are_refused() {
        let error = Config::with_overrides("[stream]\nspeed = 2").expect_err("an unknown field");
        assert!(error.message.contains("speed"), "{error}");
        let error = Config::with_overrides("[weather]\nrain = 1").expect_err("an unknown table");
        assert!(error.message.contains("weather"), "{error}");
        // The check of the whole configuration runs after the change.
        assert_eq!(
            Config::with_overrides("[director]\nmin_hold_frames = 61")
                .expect_err("a bad weight")
                .message,
            "director.min_hold_frames is 61, it must not be above director.max_hold_frames (60)"
        );
        assert!(Config::with_overrides("stream = [").is_err());
    }
}
