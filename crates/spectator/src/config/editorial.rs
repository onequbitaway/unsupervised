//! The editorial data: the weights of the director, the rules of the
//! stories, the chapters of a life story and the words for events.

use std::collections::BTreeMap;

use serde::Deserialize;

/// How the director scores events and chooses shots.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectorConfig {
    /// Lowest drama score of an event that the director counts.
    pub min_drama: u8,
    /// Frames for which an event counts.
    pub fresh_frames: u64,
    /// Percent that each more event around the same subject adds.
    pub gather_percent: u64,
    /// Greatest sum of that addition, in percent.
    pub gather_max_percent: u64,
    /// Percent of the weight of an event that a resident gets when it is
    /// not the actor.
    pub party_share_percent: u64,
    /// Percent of the weight of an event that its site gets.
    pub place_share_percent: u64,
    /// Percent of the weight of an event that its district gets.
    pub zone_share_percent: u64,
    /// Lowest score of a subject that gets a shot.
    pub cut_score: u64,
    /// Below this score the subject on air loses the shot.
    pub keep_score: u64,
    /// Frames that a shot stays at least.
    pub min_hold_frames: u64,
    /// Frames that a shot stays at most when another subject can take it.
    pub max_hold_frames: u64,
    /// Percent of the score on air that another subject needs to take the
    /// shot.
    pub switch_percent: u64,
    /// Lowest drama score of breaking news.
    pub breaking_drama: u8,
    /// Frames for which breaking news holds the shot against other
    /// breaking news that is not more dramatic.
    pub breaking_hold_frames: u64,
    /// Percent that the score of a subject loses just after it left the
    /// air.
    pub repeat_penalty_percent: u64,
    /// Percent that another subject in the district that just left the air
    /// loses.
    pub zone_penalty_percent: u64,
    /// Percent that the score of a subject loses when the event that
    /// speaks for it has the kind of a shot that started just now.
    pub kind_penalty_percent: u64,
    /// Frames after which a subject that left the air has its full score
    /// again.
    pub variety_frames: u64,
    /// Frames for which breaking news of one kind keeps other events of
    /// that kind from cutting in.
    pub breaking_kind_frames: u64,
    /// Frames that a wide view stays at least.
    pub wide_min_frames: u64,
    /// Frames after which a wide view changes to the next one.
    pub wide_hold_frames: u64,
    /// Lowest drama score of an event that gets a close view.
    pub close_drama: u8,
    /// Subjects that the director scores at the same time.
    pub max_subjects: usize,
}

/// How the service reads the data of an event.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactsConfig {
    /// Fields of the data that name a resident.
    pub resident_fields: Vec<String>,
    /// Fields that name the site of an event.
    pub site_fields: Vec<String>,
    /// Fields that name the city of an event.
    pub city_fields: Vec<String>,
    /// Residents of one event that the service uses.
    pub max_residents: usize,
    /// Names that one `{name:field}` slot of a line shows.
    pub slot_names: usize,
    /// Longest text of one `{word:field}` slot, in characters.
    pub word_chars: usize,
    /// Longest name in a line, in characters.
    pub name_chars: usize,
    /// Longest text of a resident in a line, in characters.
    pub text_chars: usize,
    /// Longest line, in characters.
    pub line_chars: usize,
}

/// The rule of one kind of story.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryKind {
    /// Name of the kind, for example `crime`.
    pub kind: String,
    /// The word that viewers see for the kind.
    pub label: String,
    /// An event joins the kind when its kind starts with one of these.
    pub prefixes: Vec<String>,
    /// Lists of data fields. The first list whose fields are all in the
    /// data names the thread of the event.
    pub threads: Vec<Vec<String>>,
    /// Residents that an event without a thread must share with a story to
    /// join it. 0: only a thread joins events.
    pub share: usize,
    /// A story takes an event by its residents only when the story had an
    /// event in the last frames.
    pub link_frames: u64,
    /// Lowest drama score of an event that opens a story.
    pub open_drama: u8,
    /// Viewers see the story from this number of events.
    pub min_events: usize,
    /// The headline, with `{lead}`, `{latest}` and `{count}`.
    pub headline: String,
    /// Data fields with the values that an event must have there. An event
    /// without the field passes.
    #[serde(default)]
    pub only: BTreeMap<String, Vec<String>>,
}

/// The chapters of a life story: the starts of the event kinds of each.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaptersConfig {
    /// The resident came into the world.
    pub born: Vec<String>,
    /// The resident left the world.
    pub death: Vec<String>,
    /// Crimes, arrests and trials.
    pub crimes: Vec<String>,
    /// Public offices and powers.
    pub offices: Vec<String>,
    /// Jobs and businesses.
    pub jobs: Vec<String>,
    /// Homes and travel.
    pub homes: Vec<String>,
    /// Contracts, loans, marriages and organizations.
    pub deals: Vec<String>,
}

/// The words for each kind of event: a template for each kind.
pub type Phrases = BTreeMap<String, String>;
