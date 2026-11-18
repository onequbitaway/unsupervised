//! The limits of the spectator service.
//!
//! The numbers live in `data/spectator.toml`. This module only reads them.

use std::fmt;

use serde::Deserialize;

/// The built-in limits.
const BUILT_IN: &str = include_str!("../data/spectator.toml");

/// The limits of the spectator service. [`Config::default`] gives the values
/// of `data/spectator.toml`.
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
}

/// Limits of the event feed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedConfig {
    /// Public events that the cache keeps, with any drama score.
    pub recent_events: usize,
    /// Notable events that the cache keeps.
    pub notable_events: usize,
    /// Lowest drama score of a notable event.
    pub notable_drama: u8,
    /// Lowest drama score of a headline.
    pub headline_drama: u8,
    /// Events in one answer of `/v1/feed` when the query has no `limit`.
    pub page_default: usize,
    /// Greatest `limit` of `/v1/feed`.
    pub page_max: usize,
}

/// Limits of one viewer connection.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocketConfig {
    /// Messages that wait for one viewer.
    pub client_queue: usize,
    /// Frames that the task of one viewer can fall behind.
    pub frame_buffer: usize,
    /// Zones that one viewer can subscribe to.
    pub max_zones: usize,
    /// Greatest size of a message from a viewer, in bytes.
    pub max_message_bytes: usize,
    /// Headlines in one `frame` message.
    pub headlines_per_frame: usize,
    /// Events in one `delta` message.
    pub zone_events_per_frame: usize,
    /// Real milliseconds that one message can take to leave.
    pub send_timeout_ms: u64,
}

/// Limits of resident profiles.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// Events in one profile.
    pub events: usize,
    /// Residents that left the world and still have a profile.
    pub departed: usize,
}

/// Limits of the zone maps.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapConfig {
    /// Frames after which a request reads the tiles and sites of a zone
    /// again.
    pub refresh_frames: u64,
}

/// How the cache starts.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartConfig {
    /// Real milliseconds before the cache tries again to read the world.
    pub retry_ms: u64,
}

/// Limits of the published state.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedConfig {
    /// Greatest number of series (kind and key) that the cache keeps.
    pub max_series: usize,
    /// Greatest length of a key, in bytes.
    pub max_key_bytes: usize,
    /// The kind that `/v1/stats/economy` serves.
    pub stats_kind: String,
    /// The kind that `/v1/leaderboards/{board}` serves.
    pub leaderboard_kind: String,
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
    /// Reads a configuration from TOML text in the format of
    /// `data/spectator.toml` and checks it.
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

    /// Checks that the service can run with these limits.
    ///
    /// # Errors
    ///
    /// Fails when a capacity is zero, when the queue of a viewer cannot
    /// hold one frame of every subscribed zone, or when the default page is
    /// larger than the greatest page.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let fail = |message: String| Err(ConfigError { message });
        let positive = [
            ("feed.recent_events", self.feed.recent_events),
            ("feed.notable_events", self.feed.notable_events),
            ("feed.page_default", self.feed.page_default),
            ("socket.frame_buffer", self.socket.frame_buffer),
            ("socket.max_zones", self.socket.max_zones),
            ("socket.max_message_bytes", self.socket.max_message_bytes),
            ("published.max_series", self.published.max_series),
        ];
        if let Some((name, _)) = positive.iter().find(|(_, value)| *value == 0) {
            return fail(format!("{name} must be greater than zero"));
        }
        if self.socket.send_timeout_ms == 0 {
            return fail("socket.send_timeout_ms must be greater than zero".into());
        }
        if self.feed.page_default > self.feed.page_max {
            return fail(format!(
                "feed.page_default is {}, it must not be above feed.page_max ({})",
                self.feed.page_default, self.feed.page_max
            ));
        }
        // One frame of a viewer is one `delta` per zone and one `frame`
        // message. A resynchronization also needs room for the `world`
        // message.
        let needed = self.socket.max_zones.saturating_add(2);
        if self.socket.client_queue < needed {
            return fail(format!(
                "socket.client_queue is {}, it must be at least socket.max_zones + 2 ({needed})",
                self.socket.client_queue
            ));
        }
        for (name, kind) in [
            ("published.stats_kind", &self.published.stats_kind),
            ("published.leaderboard_kind", &self.published.leaderboard_kind),
        ] {
            if !kind.ends_with(crate::cache::PUBLISHED_SUFFIX) {
                return fail(format!(
                    "{name} is `{kind}`, it must end in `{}`",
                    crate::cache::PUBLISHED_SUFFIX
                ));
            }
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Config {
        // The text is a file of this crate, read at compile time. The test
        // `the_built_in_limits_are_valid` proves that it parses and is valid.
        Config::from_toml(BUILT_IN).expect("data/spectator.toml is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_limits_are_valid() {
        let config = Config::from_toml(BUILT_IN).expect("the built-in limits parse");
        assert_eq!(config, Config::default());
        assert_eq!(config.feed.headline_drama, 71);
        assert_eq!(config.published.stats_kind, "finance.stats_published");
    }

    #[test]
    fn bad_limits_are_refused() {
        let message = |change: fn(&mut Config)| -> String {
            let mut config = Config::default();
            change(&mut config);
            config.validate().expect_err("the limits are bad").message
        };
        assert_eq!(
            message(|c| c.socket.client_queue = c.socket.max_zones + 1),
            "socket.client_queue is 33, it must be at least socket.max_zones + 2 (34)"
        );
        assert_eq!(
            message(|c| c.feed.recent_events = 0),
            "feed.recent_events must be greater than zero"
        );
        assert_eq!(
            message(|c| c.feed.page_default = c.feed.page_max + 1),
            "feed.page_default is 501, it must not be above feed.page_max (500)"
        );
        assert_eq!(
            message(|c| c.published.stats_kind = "finance.stats".into()),
            "published.stats_kind is `finance.stats`, it must end in `_published`"
        );
        assert_eq!(
            message(|c| c.socket.send_timeout_ms = 0),
            "socket.send_timeout_ms must be greater than zero"
        );
    }

    #[test]
    fn unknown_and_absent_fields_are_refused() {
        let unknown = BUILT_IN.replace("[map]", "[map]\nsurprise = 1");
        let error = Config::from_toml(&unknown).expect_err("an unknown field");
        assert!(error.message.contains("surprise"), "{error}");

        let absent = BUILT_IN.replace("refresh_frames = 60", "");
        let error = Config::from_toml(&absent).expect_err("an absent field");
        assert!(error.message.contains("refresh_frames"), "{error}");
    }
}
