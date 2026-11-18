//! The funnel detector: many young residents that give value to the same
//! receiver.
//!
//! A new resident has nothing. But it can earn a little at once, for
//! example with a starter shift. An owner that makes many residents and
//! lets each one give its first earnings to one resident farms the
//! starter income. The detector finds that pattern in the public events
//! of the world. It raises a flag with the evidence for an administrator.
//! It never punishes.
//!
//! The detector counts only gifts: events that move value one way, from
//! a resident that is young. A sale, a wage or an order on the exchange
//! is not a gift, and a gift of an old resident does not count.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use unsup_protocol::{OwnerId, ResidentId};

/// How the detector reads one kind of public event as a gift.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GiftEvent {
    /// Kind of the event, for example `life.gift_given`.
    pub kind: String,
    /// Field of the event data with the id of the giver. Empty: the
    /// actor of the event.
    #[serde(default)]
    pub from: String,
    /// Field of the event data with the id of the receiver.
    pub to: String,
    /// Field with the cents of money. Empty: the event moves no money.
    #[serde(default)]
    pub money: String,
    /// Field with the units of goods. Empty: the event moves no goods.
    #[serde(default)]
    pub goods: String,
    /// Field with the name of the good, for the evidence. Empty: none.
    #[serde(default)]
    pub item: String,
}

/// The thresholds of the funnel detector. The values come from the
/// `[abuse.funnel]` table of the gateway configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunnelConfig {
    /// False turns the detector off.
    pub enabled: bool,
    /// A resident is young for this many game minutes after it appeared.
    pub young_minutes: u64,
    /// The detector looks at the gifts of the last this many game
    /// minutes.
    pub window_minutes: u64,
    /// Number of young givers to one receiver that raises a flag.
    pub min_givers: usize,
    /// Least sum of money of the gifts, in cents. 0: the sum does not
    /// count.
    pub min_money: i64,
    /// Least sum of goods of the gifts, in units. 0: the sum does not
    /// count. A flag needs the money or the goods, not both.
    pub min_goods: i64,
    /// Greatest number of flags that the list keeps.
    pub max_flags: usize,
    /// Greatest number of gifts in the evidence of one flag.
    pub max_evidence: usize,
    /// Greatest number of gifts and of residents that the detector keeps
    /// in memory.
    pub max_tracked: usize,
    /// The events that are gifts.
    pub events: Vec<GiftEvent>,
}

/// One gift of a young resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Gift {
    /// Frame of the event.
    pub frame: u64,
    /// The giver.
    pub from: ResidentId,
    /// The receiver.
    pub to: ResidentId,
    /// Cents of money.
    pub money: i64,
    /// Units of goods.
    pub goods: i64,
    /// Name of the good.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Game minutes between the appearance of the giver and the gift.
    pub age_minutes: u64,
}

/// What one giver gave to the receiver of a flag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Giver {
    /// The giver.
    pub resident: ResidentId,
    /// The owner of the giver, when the store still knows it.
    pub owner: Option<OwnerId>,
    /// Game minutes between the appearance of the giver and its first
    /// gift.
    pub age_minutes: u64,
    /// Number of gifts.
    pub gifts: u64,
    /// Sum of the money, in cents.
    pub money: i64,
    /// Sum of the goods, in units.
    pub goods: i64,
}

/// A finding of the detector, with its evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Flag {
    /// Id of the flag, from 1.
    pub id: u64,
    /// The kind of finding: `funnel`.
    pub kind: &'static str,
    /// The resident that got the gifts.
    pub receiver: ResidentId,
    /// The owner of the receiver, when the store still knows it.
    pub receiver_owner: Option<OwnerId>,
    /// Frame of the first gift of the evidence.
    pub first_frame: u64,
    /// Frame of the last gift.
    pub last_frame: u64,
    /// Sum of the money of all gifts, in cents.
    pub money: i64,
    /// Sum of the goods of all gifts, in units.
    pub goods: i64,
    /// The givers, in the order of their first gift.
    pub givers: Vec<Giver>,
    /// The newest gifts, oldest first.
    pub evidence: Vec<Gift>,
    /// True when an administrator looked at the flag and closed it.
    pub dismissed: bool,
}

impl Flag {
    fn add(&mut self, gift: &Gift, max_evidence: usize) {
        self.last_frame = gift.frame;
        self.money = self.money.saturating_add(gift.money);
        self.goods = self.goods.saturating_add(gift.goods);
        match self.givers.iter_mut().find(|giver| giver.resident == gift.from) {
            Some(giver) => {
                giver.gifts += 1;
                giver.money = giver.money.saturating_add(gift.money);
                giver.goods = giver.goods.saturating_add(gift.goods);
            }
            None => self.givers.push(Giver {
                resident: gift.from,
                owner: None,
                age_minutes: gift.age_minutes,
                gifts: 1,
                money: gift.money,
                goods: gift.goods,
            }),
        }
        self.evidence.push(gift.clone());
        if self.evidence.len() > max_evidence.max(1) {
            self.evidence.remove(0);
        }
    }
}

/// The detector. It is plain state: the watcher of the gateway feeds it
/// the events.
#[derive(Debug)]
pub(crate) struct Funnel {
    config: FunnelConfig,
    /// The frame at which each known resident appeared.
    born: HashMap<ResidentId, u64>,
    /// The gifts of young residents in the window, for each receiver.
    gifts: HashMap<ResidentId, VecDeque<Gift>>,
    /// Number of gifts in `gifts`.
    held: usize,
    flags: Vec<Flag>,
    last_flag: u64,
}

impl Funnel {
    pub(crate) fn new(config: FunnelConfig) -> Funnel {
        Funnel {
            config,
            born: HashMap::new(),
            gifts: HashMap::new(),
            held: 0,
            flags: Vec::new(),
            last_flag: 0,
        }
    }

    /// Returns how the detector reads events of `kind`, when they are
    /// gifts.
    pub(crate) fn rule(&self, kind: &str) -> Option<&GiftEvent> {
        if !self.config.enabled {
            return None;
        }
        self.config.events.iter().find(|event| event.kind == kind)
    }

    /// Notes that a resident appeared at `frame`.
    pub(crate) fn appeared(&mut self, resident: ResidentId, frame: u64) {
        if self.born.len() >= self.config.max_tracked.max(1) && !self.born.contains_key(&resident) {
            // Residents that are no longer young say nothing: a gift of
            // one of them makes the watcher ask the simulation again.
            let young = self.config.young_minutes;
            self.born.retain(|_, born| frame.saturating_sub(*born) < young);
            if self.born.len() >= self.config.max_tracked.max(1) {
                return;
            }
        }
        self.born.insert(resident, frame);
    }

    /// Notes that a resident left the world.
    pub(crate) fn left(&mut self, resident: ResidentId) {
        self.born.remove(&resident);
    }

    /// Returns true when the detector knows when `resident` appeared.
    pub(crate) fn knows(&self, resident: ResidentId) -> bool {
        self.born.contains_key(&resident)
    }

    /// Forgets what it saw: the world went back to an earlier frame. The
    /// flags stay, as history.
    pub(crate) fn rolled_back(&mut self) {
        self.born.clear();
        self.gifts.clear();
        self.held = 0;
    }

    /// Takes one gift. Returns the id of the flag that the gift raised
    /// or extended.
    pub(crate) fn gift(
        &mut self,
        frame: u64,
        from: ResidentId,
        to: ResidentId,
        (money, goods): (i64, i64),
        item: Option<String>,
    ) -> Option<u64> {
        let config = &self.config;
        if !config.enabled || from == to || (money <= 0 && goods <= 0) {
            return None;
        }
        let age_minutes = frame.saturating_sub(*self.born.get(&from)?);
        if age_minutes >= config.young_minutes {
            return None;
        }
        let gift =
            Gift { frame, from, to, money: money.max(0), goods: goods.max(0), item, age_minutes };
        let (window, max_evidence) = (config.window_minutes, config.max_evidence);

        if let Some(flag) =
            self.flags.iter_mut().find(|flag| flag.receiver == to && !flag.dismissed)
        {
            flag.add(&gift, max_evidence);
            return Some(flag.id);
        }

        let recent = self.gifts.entry(to).or_default();
        while recent.front().is_some_and(|old| old.frame.saturating_add(window) < frame) {
            recent.pop_front();
            self.held -= 1;
        }
        recent.push_back(gift);
        self.held += 1;

        let mut givers: Vec<ResidentId> = recent.iter().map(|gift| gift.from).collect();
        givers.sort_unstable();
        givers.dedup();
        let money: i64 = recent.iter().map(|gift| gift.money).sum();
        let goods: i64 = recent.iter().map(|gift| gift.goods).sum();
        let config = &self.config;
        let enough = (config.min_money > 0 && money >= config.min_money)
            || (config.min_goods > 0 && goods >= config.min_goods)
            || (config.min_money <= 0 && config.min_goods <= 0);
        if givers.len() < config.min_givers.max(1) || !enough {
            self.limit_memory(frame);
            return None;
        }

        // The gifts move from the window into the flag.
        let gifts = self.gifts.remove(&to).unwrap_or_default();
        self.held -= gifts.len();
        self.last_flag += 1;
        let mut flag = Flag {
            id: self.last_flag,
            kind: "funnel",
            receiver: to,
            receiver_owner: None,
            first_frame: gifts.front().map_or(frame, |gift| gift.frame),
            last_frame: frame,
            money: 0,
            goods: 0,
            givers: Vec::new(),
            evidence: Vec::new(),
            dismissed: false,
        };
        for gift in &gifts {
            flag.add(gift, max_evidence);
        }
        if self.flags.len() >= self.config.max_flags.max(1) {
            // A flag that was dismissed goes first, else the oldest.
            let at = self.flags.iter().position(|flag| flag.dismissed).unwrap_or(0);
            self.flags.remove(at);
        }
        self.flags.push(flag);
        Some(self.last_flag)
    }

    /// Keeps the number of gifts in memory below the limit.
    fn limit_memory(&mut self, frame: u64) {
        let max = self.config.max_tracked.max(1);
        if self.held <= max {
            return;
        }
        let window = self.config.window_minutes;
        self.gifts.retain(|_, recent| {
            recent.retain(|gift| gift.frame.saturating_add(window) >= frame);
            !recent.is_empty()
        });
        self.held = self.gifts.values().map(VecDeque::len).sum();
        while self.held > max {
            // The receiver whose last gift is the oldest goes.
            let oldest = self
                .gifts
                .iter()
                .min_by_key(|(receiver, recent)| (recent.back().map(|gift| gift.frame), **receiver))
                .map(|(receiver, _)| *receiver);
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(recent) = self.gifts.remove(&oldest) {
                self.held -= recent.len();
            }
        }
    }

    /// Returns the flags, the newest first.
    pub(crate) fn flags(&self) -> Vec<Flag> {
        self.flags.iter().rev().cloned().collect()
    }

    /// Returns the residents of a flag whose owner is not known yet: the
    /// receiver first.
    pub(crate) fn without_owner(&self, id: u64) -> Vec<ResidentId> {
        let Some(flag) = self.flags.iter().find(|flag| flag.id == id) else {
            return Vec::new();
        };
        let receiver = flag.receiver_owner.is_none().then_some(flag.receiver);
        let givers =
            flag.givers.iter().filter(|giver| giver.owner.is_none()).map(|giver| giver.resident);
        receiver.into_iter().chain(givers).collect()
    }

    /// Writes the owner of a resident into a flag.
    pub(crate) fn set_owner(&mut self, id: u64, resident: ResidentId, owner: OwnerId) {
        let Some(flag) = self.flags.iter_mut().find(|flag| flag.id == id) else {
            return;
        };
        if flag.receiver == resident {
            flag.receiver_owner = Some(owner);
        }
        for giver in flag.givers.iter_mut().filter(|giver| giver.resident == resident) {
            giver.owner = Some(owner);
        }
    }

    /// Closes a flag. Returns false when no open flag has this id.
    pub(crate) fn dismiss(&mut self, id: u64) -> bool {
        match self.flags.iter_mut().find(|flag| flag.id == id && !flag.dismissed) {
            Some(flag) => {
                flag.dismissed = true;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use unsup_protocol::ResidentId;

    use super::{Funnel, FunnelConfig};
    use crate::config::GatewayConfig;

    const RECEIVER: ResidentId = ResidentId(100);

    fn config(toml: &str) -> FunnelConfig {
        let text = format!("[abuse.funnel]\nyoung_minutes = 100\nwindow_minutes = 50\n{toml}");
        GatewayConfig::from_toml(&text).expect("a configuration").abuse.funnel
    }

    /// A detector in which the residents 1 to 9 appeared at frame 0.
    fn detector(toml: &str) -> Funnel {
        let mut funnel = Funnel::new(config(toml));
        for id in 1..=9 {
            funnel.appeared(ResidentId(id), 0);
        }
        funnel
    }

    fn money(funnel: &mut Funnel, frame: u64, from: u64, cents: i64) -> Option<u64> {
        funnel.gift(frame, ResidentId(from), RECEIVER, (cents, 0), None)
    }

    #[test]
    fn the_third_young_giver_raises_a_flag_with_the_evidence() {
        let mut funnel = detector("min_givers = 3");
        assert_eq!(money(&mut funnel, 10, 1, 500), None);
        // The same giver again is still one giver.
        assert_eq!(money(&mut funnel, 11, 1, 100), None);
        assert_eq!(money(&mut funnel, 12, 2, 300), None);
        assert_eq!(funnel.flags(), []);
        assert_eq!(money(&mut funnel, 13, 3, 200), Some(1));

        let flags = funnel.flags();
        assert_eq!(flags.len(), 1);
        let flag = &flags[0];
        assert_eq!((flag.id, flag.kind, flag.receiver), (1, "funnel", RECEIVER));
        assert_eq!((flag.first_frame, flag.last_frame), (10, 13));
        assert_eq!((flag.money, flag.goods, flag.dismissed), (1100, 0, false));
        let givers: Vec<_> =
            flag.givers.iter().map(|g| (g.resident.0, g.age_minutes, g.gifts, g.money)).collect();
        assert_eq!(givers, [(1, 10, 2, 600), (2, 12, 1, 300), (3, 13, 1, 200)]);
        let frames: Vec<u64> = flag.evidence.iter().map(|gift| gift.frame).collect();
        assert_eq!(frames, [10, 11, 12, 13]);

        // A later gift goes into the same flag.
        assert_eq!(money(&mut funnel, 20, 4, 50), Some(1));
        let flag = &funnel.flags()[0];
        assert_eq!((flag.money, flag.givers.len(), flag.last_frame), (1150, 4, 20));
    }

    #[test]
    fn gifts_of_old_or_unknown_residents_do_not_count() {
        let mut funnel = detector("min_givers = 2");
        // Nobody told the detector when resident 50 appeared.
        assert!(!funnel.knows(ResidentId(50)));
        assert_eq!(money(&mut funnel, 5, 50, 500), None);
        assert_eq!(money(&mut funnel, 60, 1, 500), None);
        assert_eq!(funnel.flags(), [], "one giver that counts, and one that does not");
        // One frame before the limit, a resident is young.
        assert_eq!(money(&mut funnel, 99, 2, 500), Some(1));

        // At frame 100 the residents are no longer young.
        let other = ResidentId(300);
        for from in [1, 2, 3] {
            assert_eq!(funnel.gift(100, ResidentId(from), other, (500, 0), None), None);
        }
        assert_eq!(funnel.flags().len(), 1);
    }

    #[test]
    fn a_gift_leaves_the_window_after_its_time() {
        let mut funnel = detector("min_givers = 3");
        assert_eq!(money(&mut funnel, 10, 1, 1), None);
        assert_eq!(money(&mut funnel, 20, 2, 1), None);
        // Frame 61: the gift of frame 10 is more than 50 frames old.
        assert_eq!(money(&mut funnel, 61, 3, 1), None);
        // Frame 70: the gifts of the frames 20 and 61 are in the window.
        assert_eq!(money(&mut funnel, 70, 4, 1), Some(1));
        assert_eq!(funnel.flags()[0].first_frame, 20);
    }

    #[test]
    fn the_sum_must_reach_the_money_or_the_goods_threshold() {
        let mut funnel = detector("min_givers = 2\nmin_money = 1000\nmin_goods = 10");
        assert_eq!(money(&mut funnel, 1, 1, 400), None);
        assert_eq!(money(&mut funnel, 2, 2, 400), None, "two givers, but 800 cents");
        assert_eq!(money(&mut funnel, 3, 3, 200), Some(1), "1,000 cents");

        // Goods alone reach their own threshold.
        let other = ResidentId(200);
        let goods = |funnel: &mut Funnel, frame, from, units| {
            funnel.gift(frame, ResidentId(from), other, (0, units), Some("grain".into()))
        };
        assert_eq!(goods(&mut funnel, 4, 4, 6), None);
        assert_eq!(goods(&mut funnel, 5, 5, 4), Some(2));
        let flag = &funnel.flags()[0];
        assert_eq!((flag.receiver, flag.goods, flag.money), (other, 10, 0));
        assert_eq!(flag.evidence[0].item.as_deref(), Some("grain"));
    }

    #[test]
    fn a_gift_without_value_or_to_oneself_is_nothing() {
        let mut funnel = detector("min_givers = 1");
        assert_eq!(funnel.gift(1, ResidentId(1), RECEIVER, (0, 0), None), None);
        assert_eq!(funnel.gift(1, ResidentId(1), RECEIVER, (-5, -1), None), None);
        assert_eq!(funnel.gift(1, ResidentId(1), ResidentId(1), (500, 0), None), None);
        assert_eq!(funnel.flags(), []);
        assert_eq!(money(&mut funnel, 1, 1, 1), Some(1));
    }

    #[test]
    fn a_dismissed_flag_closes_and_a_new_funnel_raises_a_new_flag() {
        let mut funnel = detector("min_givers = 2");
        money(&mut funnel, 1, 1, 10);
        assert_eq!(money(&mut funnel, 2, 2, 10), Some(1));
        assert!(funnel.dismiss(1));
        assert!(!funnel.dismiss(1), "it is closed already");
        assert!(!funnel.dismiss(7), "no such flag");
        // The gifts of the closed flag do not count again.
        assert_eq!(money(&mut funnel, 3, 3, 10), None);
        assert_eq!(money(&mut funnel, 4, 4, 10), Some(2));
        let flags = funnel.flags();
        assert_eq!(
            flags.iter().map(|f| (f.id, f.dismissed)).collect::<Vec<_>>(),
            [(2, false), (1, true)]
        );
    }

    #[test]
    fn a_rollback_forgets_the_window_and_keeps_the_flags() {
        let mut funnel = detector("min_givers = 2");
        money(&mut funnel, 1, 1, 10);
        funnel.rolled_back();
        assert!(!funnel.knows(ResidentId(2)));
        funnel.appeared(ResidentId(2), 0);
        assert_eq!(money(&mut funnel, 2, 2, 10), None, "the gift of frame 1 is forgotten");
    }

    #[test]
    fn the_lists_have_limits() {
        let mut funnel = detector("min_givers = 2\nmax_flags = 2\nmax_evidence = 3");
        for receiver in [201, 202, 203] {
            for from in [1, 2] {
                funnel.gift(1, ResidentId(from), ResidentId(receiver), (1, 0), None);
            }
        }
        let receivers: Vec<u64> = funnel.flags().iter().map(|flag| flag.receiver.0).collect();
        assert_eq!(receivers, [203, 202], "the oldest flag made room");
        for from in 3..=6 {
            funnel.gift(2, ResidentId(from), ResidentId(203), (1, 0), None);
        }
        let flag = &funnel.flags()[0];
        assert_eq!((flag.givers.len(), flag.evidence.len()), (6, 3));
        assert_eq!(flag.evidence.iter().map(|g| g.from.0).collect::<Vec<_>>(), [4, 5, 6]);
    }

    #[test]
    fn the_memory_has_a_limit() {
        // Four residents and four gifts at most.
        let mut funnel = detector("min_givers = 3\nmax_tracked = 4");
        assert!(funnel.knows(ResidentId(4)) && !funnel.knows(ResidentId(5)));
        // Single gifts to many receivers.
        for receiver in 300..310 {
            funnel.gift(receiver - 299, ResidentId(1), ResidentId(receiver), (1, 0), None);
        }
        assert_eq!(funnel.held, 4);
        // The gifts to the last receivers stayed: two more givers make
        // the funnel complete.
        assert_eq!(funnel.gift(11, ResidentId(2), ResidentId(309), (1, 0), None), None);
        assert_eq!(funnel.gift(11, ResidentId(3), ResidentId(309), (1, 0), None), Some(1));
        // The gift to the first receiver went: two more givers are not
        // enough there.
        assert_eq!(funnel.gift(12, ResidentId(2), ResidentId(300), (1, 0), None), None);
        assert_eq!(funnel.gift(12, ResidentId(3), ResidentId(300), (1, 0), None), None);
        // A resident that is no longer young makes room for a new one.
        funnel.appeared(ResidentId(50), 500);
        assert!(funnel.knows(ResidentId(50)) && !funnel.knows(ResidentId(1)));
    }
}
