//! Ids of everything that the crime module defines or reads: parameters,
//! attributes, conditions, faucets and sinks. They are resolved once, when
//! the plugin is built.

use unsup_kernel::{
    AttrDef, AttrId, AttrShow, Builder, CondDef, CondId, FaucetId, ParamId, Realm, SinkId,
};
use unsup_life::names as life;

use crate::kinds::CrimeKind;
use crate::names::{attr, cond, flow};

/// Declares the struct [`Params`] with one `ParamId` for each line.
macro_rules! params {
    ($($field:ident => $name:literal,)*) => {
        /// Ids of the parameters of `data/balance.toml` that the code reads
        /// by name.
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct Params {
            $(pub(crate) $field: ParamId,)*
        }

        impl Params {
            fn new(b: &Builder) -> Params {
                Params { $($field: b.param_id($name),)* }
            }
        }
    };
}

params! {
    chance_min => "crime.chance_min_bp",
    chance_max => "crime.chance_max_bp",
    night_from => "crime.night_from_hour",
    night_to => "crime.night_to_hour",
    night_bonus => "crime.night_bonus_bp",
    stealth_level => "crime.stealth_level_bp",
    combat_level => "crime.combat_level_bp",
    xp_success => "crime.xp_success",
    xp_fail => "crime.xp_fail",

    pickpocket_ticks => "crime.pickpocket_ticks",
    pickpocket_range => "crime.pickpocket_range",
    pickpocket_base => "crime.pickpocket_base_bp",
    pickpocket_asleep => "crime.pickpocket_asleep_bonus_bp",
    pickpocket_alert => "crime.pickpocket_alert_bp",
    pickpocket_pct => "crime.pickpocket_pct",
    pickpocket_max => "crime.pickpocket_max",

    burgle_ticks => "crime.burgle_ticks",
    burgle_base => "crime.burgle_base_bp",
    burgle_empty => "crime.burgle_empty_bonus_bp",
    burgle_occupant => "crime.burgle_occupant_malus_bp",
    burgle_tool => "crime.burgle_tool_bonus_bp",
    burgle_public => "crime.burgle_public_malus_bp",
    burgle_till_pct => "crime.burgle_till_pct",
    burgle_units_max => "crime.burgle_units_max",

    rob_ticks => "crime.rob_ticks",
    rob_range => "crime.rob_range",
    rob_base => "crime.rob_base_bp",
    force_asleep => "crime.force_asleep_bonus_bp",
    rob_cash_pct => "crime.rob_cash_pct",
    rob_units_max => "crime.rob_units_max",
    rob_injury => "crime.rob_injury",
    rob_fail_injury => "crime.rob_fail_injury",

    assault_ticks => "crime.assault_ticks",
    assault_range => "crime.assault_range",
    assault_base => "crime.assault_base_bp",
    assault_damage => "crime.assault_damage",
    assault_level_damage => "crime.assault_level_damage",
    assault_fail_injury => "crime.assault_fail_injury",

    vandalize_ticks => "crime.vandalize_ticks",
    vandalize_base => "crime.vandalize_base_bp",
    vandalize_stock_pct => "crime.vandalize_stock_pct",
    vandalize_value => "crime.vandalize_value_bp",
    vandalize_damage_max => "crime.vandalize_damage_max_bp",
    repair_ticks => "crime.repair_ticks",
    repair_cost => "crime.repair_cost_bp",
    repair_xp => "crime.repair_xp",

    fence_ticks => "crime.fence_ticks",
    fence_share => "crime.fence_bp",
    fence_base_share => "crime.fence_base_bp",

    bribe_ticks => "crime.bribe_ticks",
    bribe_range => "crime.bribe_range",
    bribe_note_max => "crime.bribe_note_max",

    hot_ticks => "crime.hot_ticks",
    hot_lots_max => "crime.hot_lots_max",

    witness_radius => "crime.witness_radius",
    witness_max => "crime.witness_max",
    identify_base => "crime.identify_base_bp",
    identify_stealth => "crime.identify_stealth_bp",
    identify_night => "crime.identify_night_malus_bp",
    identify_distance => "crime.identify_distance_bp",
    identify_fail => "crime.identify_fail_bonus_bp",
    identify_force => "crime.identify_force_bonus_bp",
    identify_asleep => "crime.identify_asleep_malus_bp",
    known_max => "crime.known_max",

    evidence_victim => "crime.evidence_victim",
    evidence_witness => "crime.evidence_witness",
    evidence_investigate => "crime.evidence_investigate",
    evidence_investigate_max => "crime.evidence_investigate_max",
    evidence_search => "crime.evidence_search",
    evidence_caught => "crime.evidence_caught",
    evidence_watch => "crime.evidence_watch",
    warrant_evidence => "crime.warrant_evidence",
    charge_evidence => "crime.charge_evidence",
    convict_evidence => "crime.convict_evidence",
    trace_ticks => "crime.trace_ticks",
    traces_max => "crime.traces_max",

    patrol_ticks => "crime.patrol_ticks",
    patrol_max_ticks => "crime.patrol_max_ticks",
    patrol_radius => "crime.patrol_radius",
    patrol_detect => "crime.patrol_detect_bp",
    patrol_stealth => "crime.patrol_stealth_bp",
    patrol_xp => "crime.patrol_xp_per_hour",
    patrol_list_max => "crime.patrol_list_max",
    investigate_ticks => "crime.investigate_ticks",
    investigate_radius => "crime.investigate_radius",
    investigate_base => "crime.investigate_base_bp",
    investigate_level => "crime.investigate_level_bp",
    investigate_stealth => "crime.investigate_stealth_bp",
    investigate_xp => "crime.investigate_xp",
    search_ticks => "crime.search_ticks",
    search_range => "crime.search_range",
    search_xp => "crime.search_xp",
    arrest_ticks => "crime.arrest_ticks",
    arrest_range => "crime.arrest_range",
    arrest_xp => "crime.arrest_xp",
    caught_ticks => "crime.caught_ticks",
    hold_ticks => "crime.hold_ticks",
    contraband_illegal => "crime.contraband_illegal",

    trial_ticks => "crime.trial_ticks",
    trial_bench_ticks => "crime.trial_bench_ticks",
    jury_size => "crime.jury_size",
    jury_quorum => "crime.jury_quorum",
    jury_draws => "crime.jury_draws",
    fine_due_ticks => "crime.fine_due_ticks",
    fine_per_jail_tick => "crime.fine_cents_per_jail_tick",
    fine_jail_max => "crime.fine_jail_max_ticks",
    conviction_standing => "crime.conviction_standing",
    court_xp => "crime.court_xp",

    jail_meal_below => "crime.jail_meal_below",
    jail_meal_food => "crime.jail_meal_food",
    jail_rest_below => "crime.jail_rest_below",
    jail_rest_energy => "crime.jail_rest_energy",

    watch_detect => "crime.watch_detect_bp",
    watch_arrest => "crime.watch_arrest_bp",
    watch_contraband => "crime.watch_contraband_bp",

    case_ttl => "crime.case_ttl_ticks",
    case_keep => "crime.case_keep_ticks",
    case_list_max => "crime.case_list_max",
    heist_value => "crime.heist_value",

    bounty_min => "crime.bounty_min",
    bounty_ticks => "crime.bounty_ticks",
    bounty_max_ticks => "crime.bounty_max_ticks",
    bounties_open_max => "crime.bounties_open_max",

    observe_list_max => "crime.observe_list_max",

    drama_attempt => "crime.drama.attempt",
    drama_heist => "crime.drama.heist",
    drama_warrant => "crime.drama.warrant_issued",
    drama_search => "crime.drama.search_made",
    drama_arrest => "crime.drama.arrest_made",
    drama_charged => "crime.drama.suspect_charged",
    drama_released => "crime.drama.suspect_released",
    drama_jury => "crime.drama.jury_summoned",
    drama_verdict => "crime.drama.verdict_given",
    drama_served => "crime.drama.sentence_served",
    drama_fine_paid => "crime.drama.fine_paid",
    drama_fine_defaulted => "crime.drama.fine_defaulted",
    drama_fenced => "crime.drama.goods_fenced",
    drama_repaired => "crime.drama.site_repaired",
    drama_bounty_posted => "crime.drama.bounty_posted",
    drama_bounty_paid => "crime.drama.bounty_paid",
    drama_bounty_expired => "crime.drama.bounty_expired",

    global_timeout => "kernel.global_timeout",
}

/// The sentence numbers of one kind of crime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KindParams {
    /// Cents of the fine that the engine gives.
    pub(crate) fine: ParamId,
    /// Ticks of jail that the engine gives.
    pub(crate) jail: ParamId,
    /// Greatest fine that a judge can give.
    pub(crate) fine_max: ParamId,
    /// Greatest jail time that a judge can give.
    pub(crate) jail_max: ParamId,
    /// Notoriety that a conviction adds.
    pub(crate) notoriety: ParamId,
    /// Drama score of the event of the crime.
    pub(crate) drama: ParamId,
}

impl KindParams {
    fn new(b: &Builder, kind: CrimeKind) -> KindParams {
        let id = |what: &str| b.param_id(&format!("crime.{}.{what}", kind.name()));
        KindParams {
            fine: id("fine"),
            jail: id("jail_ticks"),
            fine_max: id("fine_max"),
            jail_max: id("jail_max_ticks"),
            notoriety: id("notoriety"),
            drama: id("drama"),
        }
    }
}

/// Ids of the attributes, conditions, faucets, sinks and parameters of the
/// module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Handles {
    pub(crate) health: AttrId,
    pub(crate) food: AttrId,
    pub(crate) energy: AttrId,
    pub(crate) stealth: AttrId,
    pub(crate) combat: AttrId,
    pub(crate) investigation: AttrId,
    pub(crate) law: AttrId,
    pub(crate) labor: AttrId,
    pub(crate) standing: AttrId,
    pub(crate) notoriety: AttrId,
    pub(crate) asleep: CondId,
    pub(crate) wanted: CondId,
    pub(crate) detained: CondId,
    pub(crate) jailed: CondId,
    /// Sink of goods that vandalism destroys.
    pub(crate) destroyed: SinkId,
    /// Sink of the money that a repair costs.
    pub(crate) repair: SinkId,
    /// Sink of fenced goods.
    pub(crate) fenced_goods: SinkId,
    /// Faucet of the money that a fence pays.
    pub(crate) fence_money: FaucetId,
    pub(crate) p: Params,
    kinds: [KindParams; CrimeKind::ALL.len()],
}

impl Handles {
    /// Loads the data file and declares the attributes, the conditions,
    /// the faucet and the sinks. It is safe to call it in both passes of
    /// the builder.
    pub(crate) fn register(b: &mut Builder) -> Handles {
        b.load_data(include_str!("../data/balance.toml"));

        // Shared attributes have the definition of the module that owns
        // them. The first declaration of a name wins.
        let need = |name: &str, summary: &str| {
            AttrDef::new(name)
                .range(0, life::attr::NEED_MAX)
                .default(life::attr::NEED_MAX)
                .summary(summary)
        };
        let health = b.attr(need(life::attr::HEALTH, "Health, 0 to 1000. At 0 the resident dies."));
        let food = b.attr(need(life::attr::FOOD, "Food, 0 to 1000. At 0 the resident starves."));
        let energy =
            b.attr(need(life::attr::ENERGY, "Energy, 0 to 1000. At 0 the resident is exhausted."));
        let mut skill = |name: &str| {
            b.attr(
                AttrDef::new(unsup_life::skill_attr(name))
                    .range(0, i64::MAX)
                    .show(AttrShow::Full)
                    .summary(format!(
                        "Experience in {name}. The level is floor(sqrt(experience / 100))."
                    )),
            )
        };
        let stealth = skill("stealth");
        let combat = skill("combat");
        let investigation = skill("investigation");
        let law = skill("law");
        let labor = skill("labor");
        let standing = b.attr(AttrDef::new(attr::STANDING).range(-1_000, 1_000).default(0).summary(
            "Public reputation. Kept contracts and praise raise it. Broken contracts and blame lower it.",
        ));
        let notoriety = b.attr(
            AttrDef::new(attr::NOTORIETY)
                .range(0, i64::MAX)
                .default(0)
                .summary("Criminal fame, 0 and up. Each conviction raises it."),
        );

        let asleep = b.condition(
            CondDef::new(life::cond::ASLEEP).summary("The resident sleeps. Its sight is reduced."),
        );
        let wanted = b.condition(
            CondDef::new(cond::WANTED)
                .summary("A warrant names the resident. Police can arrest it on sight."),
        );
        let detained = b.condition(
            CondDef::new(cond::DETAINED)
                .blocking()
                .summary("The resident is under arrest and waits for a decision."),
        );
        let jailed = b.condition(
            CondDef::new(cond::JAILED)
                .blocking()
                .summary("The resident serves a sentence in jail. Only a few verbs work."),
        );

        Handles {
            health,
            food,
            energy,
            stealth,
            combat,
            investigation,
            law,
            labor,
            standing,
            notoriety,
            asleep,
            wanted,
            detained,
            jailed,
            destroyed: b.sink(flow::DESTRUCTION),
            repair: b.sink(flow::REPAIR),
            fenced_goods: b.sink(flow::EXPORT),
            fence_money: b.faucet(flow::EXPORT),
            p: Params::new(b),
            kinds: CrimeKind::ALL.map(|kind| KindParams::new(b, kind)),
        }
    }

    /// Returns the sentence numbers of a kind of crime.
    pub(crate) fn kind(&self, kind: CrimeKind) -> &KindParams {
        &self.kinds[kind.index()]
    }

    /// Returns the drama score of an event from its parameter.
    pub(crate) fn drama(&self, realm: &Realm, param: ParamId) -> u8 {
        u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(0)
    }

    /// Returns a parameter as a number of ticks, at least 1.
    pub(crate) fn ticks(&self, realm: &Realm, param: ParamId) -> u32 {
        u32::try_from(realm.param(param).max(1)).unwrap_or(u32::MAX)
    }

    /// Returns a parameter as a number of frames, 0 or more.
    pub(crate) fn frames(&self, realm: &Realm, param: ParamId) -> u64 {
        u64::try_from(realm.param(param).max(0)).unwrap_or(0)
    }

    /// Returns a parameter as a distance in tiles.
    pub(crate) fn tiles(&self, realm: &Realm, param: ParamId) -> u32 {
        u32::try_from(realm.param(param).max(0)).unwrap_or(0)
    }

    /// Returns a parameter as a count.
    pub(crate) fn count(&self, realm: &Realm, param: ParamId) -> usize {
        usize::try_from(realm.param(param).max(0)).unwrap_or(0)
    }
}
