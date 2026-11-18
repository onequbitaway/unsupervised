//! The rule filter: fixed rules from the configuration.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Deserialize;
use unsup_protocol::ResidentId;

use super::filter::{Author, FilterFuture, Refusal, Text, TextFilter};
use super::links;
use super::pattern::Pattern;
use crate::config::ConfigError;
use crate::limit::{RateLimiter, retry_after_secs};

/// What the rule filter does with a text that holds a link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkRule {
    /// A text can hold links.
    #[default]
    Allow,
    /// A text with a link is refused.
    Refuse,
    /// A text can hold links to the hosts of the list, and to no other
    /// host.
    Listed,
}

/// The rules of moderation. The values come from the `[moderation]` table
/// of the gateway configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModerationConfig {
    /// Greatest length of one text, in bytes. 0: no limit.
    pub max_text_bytes: usize,
    /// Greatest number of times that one character can stand in a row.
    /// 0: no limit.
    pub max_repeat: usize,
    /// What a text with a link gets.
    pub links: LinkRule,
    /// With [`LinkRule::Listed`]: the hosts that a link can point at. A
    /// host covers its parts: `example.org` covers `docs.example.org`.
    pub link_hosts: Vec<String>,
    /// Last parts of host names. A word such as `name.com` counts as a
    /// link when its last part is in this list.
    pub link_endings: Vec<String>,
    /// Words and phrases that no text can hold. A word matches a whole
    /// word, in upper or lower case.
    pub words: Vec<String>,
    /// Patterns that no text can hold. See the configuration file for
    /// the wildcards.
    pub patterns: Vec<String>,
    /// Texts of one resident in one minute. 0: no limit.
    pub texts_per_minute: u32,
    /// Texts that a resident can send at once after a quiet time.
    pub text_burst: u32,
    /// Verbs that carry text for other residents or for viewers. Every
    /// text argument of such a verb is speech.
    pub text_verbs: Vec<String>,
    /// Names of arguments that hold speech in every verb.
    pub text_args: Vec<String>,
}

impl ModerationConfig {
    /// Returns true when a text argument with the name `arg` of `verb` is
    /// speech: text for other residents or for viewers.
    #[must_use]
    pub fn is_speech(&self, verb: &str, arg: &str) -> bool {
        self.text_verbs.iter().any(|v| v == verb) || self.text_args.iter().any(|a| a == arg)
    }
}

/// Returns true for a character that changes the direction of the text
/// around it. Such a character can make a text read as another text.
const fn changes_direction(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Returns true for a character that takes no place on the screen. The
/// rules compare a text without these characters, so that they cannot
/// hide a word.
const fn is_invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Returns a text as the word list and the pattern list compare it: in
/// lower case, without invisible characters, with one space for each run
/// of white space.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars().filter(|c| !is_invisible(*c)) {
        if c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Returns the words of a normalized text in a row, with one space before
/// and after each. A phrase is in the text when ` phrase ` is in this.
fn spaced_words(normalized: &str) -> String {
    let mut out = String::with_capacity(normalized.len() + 2);
    out.push(' ');
    for word in normalized.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        out.push_str(word);
        out.push(' ');
    }
    out
}

/// A text filter with fixed rules: a length limit, no control characters,
/// a limit of repeated characters, a rule for links, a list of words, a
/// list of patterns, and a limit of texts for each resident.
///
/// The limit of texts counts speech only (see
/// [`TextUse::is_speech`](crate::TextUse::is_speech)), and only speech
/// that the other rules let through.
#[derive(Debug)]
pub struct RuleFilter {
    config: ModerationConfig,
    /// The single words of the list, as they are compared.
    words: HashSet<String>,
    /// The phrases of the list, each as ` word word `.
    phrases: Vec<String>,
    patterns: Vec<Pattern>,
    rate: RateLimiter<ResidentId>,
}

impl RuleFilter {
    /// Makes the filter from the rules of the configuration.
    ///
    /// # Errors
    ///
    /// Fails for a word without a letter or a digit, and for a pattern
    /// that is empty or that every text matches.
    pub fn new(config: &ModerationConfig) -> Result<RuleFilter, ConfigError> {
        let mut words = HashSet::new();
        let mut phrases = Vec::new();
        for entry in &config.words {
            let spaced = spaced_words(&normalize(entry));
            let inner = spaced.trim();
            if inner.is_empty() {
                return Err(ConfigError(format!(
                    "`moderation.words` holds `{entry}`, which has no letter and no digit"
                )));
            }
            if inner.contains(' ') {
                phrases.push(spaced);
            } else {
                words.insert(inner.to_string());
            }
        }
        let patterns = config
            .patterns
            .iter()
            .map(|pattern| Pattern::parse(&normalize(pattern)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|reason| ConfigError(format!("`moderation.patterns`: {reason}")))?;
        let rate =
            RateLimiter::new(config.texts_per_minute, Duration::from_secs(60), config.text_burst);
        Ok(RuleFilter { config: config.clone(), words, phrases, patterns, rate })
    }

    /// Checks the content of a text. The time and the author do not count.
    ///
    /// # Errors
    ///
    /// The first rule that the text breaks.
    pub fn check_content(&self, text: &str) -> Result<(), Refusal> {
        let config = &self.config;
        if config.max_text_bytes > 0 && text.len() > config.max_text_bytes {
            return Err(Refusal::new(
                "too_long",
                format!(
                    "The text has {} bytes. This server takes at most {}.",
                    text.len(),
                    config.max_text_bytes
                ),
            ));
        }
        let forbidden =
            |c: char| (c.is_control() && c != '\n' && c != '\t') || changes_direction(c);
        if text.chars().any(forbidden) {
            return Err(Refusal::new(
                "control_character",
                "The text holds a control character. Only line breaks and tabs are permitted.",
            ));
        }
        if config.max_repeat > 0 && longest_run(text) > config.max_repeat {
            return Err(Refusal::new(
                "repeated_character",
                format!(
                    "The text repeats one character more than {} times in a row.",
                    config.max_repeat
                ),
            ));
        }
        self.check_links(text)?;

        let normalized = normalize(text);
        let spaced = spaced_words(&normalized);
        let has_word = spaced.split(' ').any(|word| self.words.contains(word))
            || self.phrases.iter().any(|phrase| spaced.contains(phrase.as_str()));
        if has_word {
            return Err(Refusal::new(
                "word",
                "The text holds a word that this server does not permit.",
            ));
        }
        if !self.patterns.is_empty() {
            let chars: Vec<char> = normalized.chars().collect();
            if self.patterns.iter().any(|pattern| pattern.found_in(&chars)) {
                return Err(Refusal::new(
                    "pattern",
                    "The text matches a pattern that this server does not permit.",
                ));
            }
        }
        Ok(())
    }

    fn check_links(&self, text: &str) -> Result<(), Refusal> {
        let config = &self.config;
        if config.links == LinkRule::Allow {
            return Ok(());
        }
        // An invisible character must not split a link in two.
        let visible: String = text.chars().filter(|c| !is_invisible(*c)).collect();
        let hosts = links::hosts(&visible, &config.link_endings);
        match config.links {
            LinkRule::Refuse if !hosts.is_empty() => {
                Err(Refusal::new("link", "The text holds a link. This server takes no links."))
            }
            LinkRule::Listed
                if hosts.iter().any(|host| !links::is_listed(host, &config.link_hosts)) =>
            {
                Err(Refusal::new(
                    "link",
                    "The text holds a link to a host that this server does not list.",
                ))
            }
            _ => Ok(()),
        }
    }

    /// Takes one text from the limit of a resident at the time `now`.
    ///
    /// # Errors
    ///
    /// When the resident sent too many texts: a refusal with the time to
    /// wait.
    pub fn take_text(&self, resident: ResidentId, now: Instant) -> Result<(), Refusal> {
        self.rate.check(resident, now).map_err(|wait| Refusal {
            rule: "rate".into(),
            message: format!("Too many texts. Try again in {} seconds.", retry_after_secs(wait)),
            retry_after: Some(wait),
        })
    }
}

/// Returns the length of the longest run of one character.
fn longest_run(text: &str) -> usize {
    let (mut longest, mut run, mut last) = (0, 0, None);
    for c in text.chars() {
        run = if last == Some(c) { run + 1 } else { 1 };
        last = Some(c);
        longest = longest.max(run);
    }
    longest
}

impl TextFilter for RuleFilter {
    fn name(&self) -> &str {
        "rules"
    }

    fn check<'a>(&'a self, text: &'a Text<'a>) -> FilterFuture<'a> {
        let answer = self.check_content(text.text).and_then(|()| match text.author {
            Author::Resident(resident) if text.usage.is_speech() => {
                self.take_text(resident, Instant::now())
            }
            _ => Ok(()),
        });
        Box::pin(std::future::ready(answer))
    }
}
