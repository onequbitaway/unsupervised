//! The check of a configuration.

use std::collections::BTreeSet;

use super::{Config, ConfigError};
use crate::facts::Template;

fn fail<T>(message: String) -> Result<T, ConfigError> {
    Err(ConfigError { message })
}

impl Config {
    /// Checks that the service can run with this configuration.
    ///
    /// # Errors
    ///
    /// Fails when a capacity is zero, when the queue of a viewer cannot
    /// hold one frame of every subscribed zone, when a page default is
    /// larger than the greatest page, when a weight of the director is out
    /// of its range, when a clip is longer than the frames that the service
    /// keeps, or when a template or a story rule is not valid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.check_limits()?;
        self.check_director()?;
        self.check_words()
    }

    fn check_limits(&self) -> Result<(), ConfigError> {
        let positive = [
            ("feed.recent_events", self.feed.recent_events),
            ("feed.notable_events", self.feed.notable_events),
            ("feed.page_default", self.feed.page_default),
            ("socket.frame_buffer", self.socket.frame_buffer),
            ("socket.max_zones", self.socket.max_zones),
            ("socket.max_message_bytes", self.socket.max_message_bytes),
            ("published.max_series", self.published.max_series),
            ("stories.max_open", self.stories.max_open),
            ("stories.residents", self.stories.residents),
            ("stories.updates_per_frame", self.stories.updates_per_frame),
            ("stories.page_default", self.stories.page_default),
            ("chronicle.resident_events", self.chronicle.resident_events),
            ("chronicle.place_events", self.chronicle.place_events),
            ("chronicle.residents", self.chronicle.residents),
            ("chronicle.places", self.chronicle.places),
            ("chronicle.compact_records", self.chronicle.compact_records),
            ("chronicle.forget_one_in", self.chronicle.forget_one_in),
            ("facts.slot_names", self.facts.slot_names),
            ("director.max_subjects", self.director.max_subjects),
            ("facts.max_residents", self.facts.max_residents),
            ("facts.name_chars", self.facts.name_chars),
            ("facts.line_chars", self.facts.line_chars),
        ];
        if let Some((name, _)) = positive.iter().find(|(_, value)| *value == 0) {
            return fail(format!("{name} must be greater than zero"));
        }
        if self.socket.send_timeout_ms == 0 {
            return fail("socket.send_timeout_ms must be greater than zero".into());
        }
        for (name, default, max) in [
            ("feed", self.feed.page_default, self.feed.page_max),
            ("stories", self.stories.page_default, self.stories.page_max),
        ] {
            if default > max {
                return fail(format!(
                    "{name}.page_default is {default}, it must not be above {name}.page_max ({max})"
                ));
            }
        }
        // One frame of a viewer is one `delta` per zone, one `director`
        // message, one `stories` message and one `frame` message. A
        // resynchronization also needs room for the `world` message.
        let needed = self.socket.max_zones.saturating_add(4);
        if self.socket.client_queue < needed {
            return fail(format!(
                "socket.client_queue is {}, it must be at least socket.max_zones + 4 ({needed})",
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
        if self.stories.timeline < 2 {
            return fail("stories.timeline must be at least 2".into());
        }
        let clips = &self.clips;
        let clip_frames = clips.before_frames.saturating_add(clips.after_frames).saturating_add(1);
        if clips.window_frames > 0 && clip_frames > clips.window_frames {
            return fail(format!(
                "clips.before_frames + clips.after_frames + 1 is {clip_frames}, it must not be \
                 above clips.window_frames ({})",
                clips.window_frames
            ));
        }
        if clips.auto_pin_drama > 101 {
            return fail("clips.auto_pin_drama must not be above 101".into());
        }
        Ok(())
    }

    fn check_director(&self) -> Result<(), ConfigError> {
        let director = &self.director;
        for (name, value) in [
            ("director.fresh_frames", director.fresh_frames),
            ("director.variety_frames", director.variety_frames),
            ("director.wide_hold_frames", director.wide_hold_frames),
            ("director.cut_score", director.cut_score),
        ] {
            if value == 0 {
                return fail(format!("{name} must be greater than zero"));
            }
        }
        for (name, value) in [
            ("director.repeat_penalty_percent", director.repeat_penalty_percent),
            ("director.zone_penalty_percent", director.zone_penalty_percent),
            ("director.kind_penalty_percent", director.kind_penalty_percent),
            ("director.party_share_percent", director.party_share_percent),
            ("director.place_share_percent", director.place_share_percent),
            ("director.zone_share_percent", director.zone_share_percent),
        ] {
            if value > 100 {
                return fail(format!("{name} is {value}, it must not be above 100"));
            }
        }
        if director.switch_percent < 100 {
            return fail(format!(
                "director.switch_percent is {}, it must be at least 100",
                director.switch_percent
            ));
        }
        if director.min_hold_frames > director.max_hold_frames {
            return fail(format!(
                "director.min_hold_frames is {}, it must not be above director.max_hold_frames ({})",
                director.min_hold_frames, director.max_hold_frames
            ));
        }
        if director.keep_score > director.cut_score {
            return fail(format!(
                "director.keep_score is {}, it must not be above director.cut_score ({})",
                director.keep_score, director.cut_score
            ));
        }
        Ok(())
    }

    fn check_words(&self) -> Result<(), ConfigError> {
        for (kind, text) in &self.phrases {
            if let Err(message) = Template::parse(text) {
                return fail(format!("phrases.\"{kind}\": {message}"));
            }
        }
        let mut kinds = BTreeSet::new();
        for rule in &self.story_kinds {
            if rule.kind.is_empty() || !kinds.insert(rule.kind.as_str()) {
                return fail(format!(
                    "story `{}`: each story has its own name in `kind`",
                    rule.kind
                ));
            }
            if rule.prefixes.is_empty() || rule.prefixes.iter().any(String::is_empty) {
                return fail(format!("story `{}`: `prefixes` needs text", rule.kind));
            }
            if rule.threads.iter().any(Vec::is_empty) {
                return fail(format!("story `{}`: a list in `threads` needs a field", rule.kind));
            }
            if rule.min_events == 0 {
                return fail(format!("story `{}`: `min_events` must be at least 1", rule.kind));
            }
            for slot in crate::stories::headline_slots(&rule.headline) {
                if !matches!(slot, "lead" | "latest" | "count") {
                    return fail(format!(
                        "story `{}`: the headline has the unknown slot `{slot}`",
                        rule.kind
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(change: fn(&mut Config)) -> String {
        let mut config = Config::default();
        change(&mut config);
        config.validate().expect_err("the configuration is bad").message
    }

    #[test]
    fn bad_limits_are_refused() {
        assert_eq!(
            message(|c| c.socket.client_queue = c.socket.max_zones + 3),
            "socket.client_queue is 35, it must be at least socket.max_zones + 4 (36)"
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
        assert_eq!(
            message(|c| c.clips.window_frames = 60),
            "clips.before_frames + clips.after_frames + 1 is 61, it must not be above \
             clips.window_frames (60)"
        );
        assert_eq!(message(|c| c.stories.timeline = 1), "stories.timeline must be at least 2");
    }

    #[test]
    fn clips_can_be_turned_off() {
        let mut config = Config::default();
        config.clips.window_frames = 0;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn bad_weights_are_refused() {
        assert_eq!(
            message(|c| c.director.min_hold_frames = c.director.max_hold_frames + 1),
            "director.min_hold_frames is 61, it must not be above director.max_hold_frames (60)"
        );
        assert_eq!(
            message(|c| c.director.switch_percent = 99),
            "director.switch_percent is 99, it must be at least 100"
        );
        assert_eq!(
            message(|c| c.director.repeat_penalty_percent = 101),
            "director.repeat_penalty_percent is 101, it must not be above 100"
        );
        assert_eq!(
            message(|c| c.director.fresh_frames = 0),
            "director.fresh_frames must be greater than zero"
        );
        assert_eq!(
            message(|c| c.director.keep_score = c.director.cut_score + 1),
            "director.keep_score is 31, it must not be above director.cut_score (30)"
        );
    }

    #[test]
    fn bad_words_and_story_rules_are_refused() {
        assert_eq!(
            message(|c| {
                c.phrases.insert("x.y".into(), "{nobody}".into());
            }),
            "phrases.\"x.y\": `{nobody}` has the unknown slot `nobody`"
        );
        assert_eq!(
            message(|c| c.story_kinds[1].kind = "crime".into()),
            "story `crime`: each story has its own name in `kind`"
        );
        assert_eq!(
            message(|c| c.story_kinds[0].prefixes.clear()),
            "story `crime`: `prefixes` needs text"
        );
        assert_eq!(
            message(|c| c.story_kinds[0].headline = "{lead} and {more}".into()),
            "story `crime`: the headline has the unknown slot `more`"
        );
        assert_eq!(
            message(|c| c.story_kinds[0].min_events = 0),
            "story `crime`: `min_events` must be at least 1"
        );
    }
}
