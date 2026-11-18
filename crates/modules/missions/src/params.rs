//! The parameters of the module and the other names that it registers.
//! Every number comes from `data/balance.toml`.

use unsup_kernel::{AttrDef, AttrId, Builder, FaucetId, ParamId, Realm, SinkId};

/// The data file of the module.
const DATA: &str = include_str!("../data/balance.toml");

/// Name of the faucet that creates the reward of a sponsor mission.
pub const SPONSOR_FAUCET: &str = "sponsor.reward";
/// Name of the sink that destroys the reward of a sponsor mission that
/// nobody earned before the deadline.
pub const SPONSOR_SINK: &str = "sponsor.expired";
/// Name of the sink that destroys the cash that the end of a season cuts.
pub const SEASON_SINK: &str = "season.reset";
/// Name of the reputation attribute. The society module owns it.
pub const STANDING: &str = "rep.standing";
/// Name of the attribute for criminal fame. The crime module owns it.
pub const NOTORIETY: &str = "rep.notoriety";

macro_rules! params {
    ($($field:ident),* $(,)?) => {
        /// The ids of the parameters, the attributes, the faucet and the
        /// sinks of the module. The name of a parameter field is the name
        /// of the parameter without the prefix `missions.`.
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct Params {
            $(pub(crate) $field: ParamId,)*
            /// Ticks after which an action that waits for the global realm
            /// fails. It is a parameter of the kernel.
            pub(crate) global_timeout: ParamId,
            /// The attribute `rep.standing`.
            pub(crate) standing: AttrId,
            /// The attribute `rep.notoriety`.
            pub(crate) notoriety: AttrId,
            /// The faucet `sponsor.reward`.
            pub(crate) sponsor_faucet: FaucetId,
            /// The sink `sponsor.expired`.
            pub(crate) sponsor_sink: SinkId,
            /// The sink `season.reset`.
            pub(crate) season_sink: SinkId,
        }

        impl Params {
            /// Loads the data file, looks up every parameter and declares
            /// the shared attributes, the faucet and the sinks. It is safe
            /// to call it in both passes of the builder.
            pub(crate) fn load(b: &mut Builder) -> Params {
                b.load_data(DATA);
                // The definitions of `docs/design/game-design.md`, section
                // 3. The first declaration of a name wins, so the modules
                // that own the attributes can declare them too.
                let standing = b.attr(
                    AttrDef::new(STANDING)
                        .range(-1_000, 1_000)
                        .default(0)
                        .summary("Public reputation, -1000 to 1000."),
                );
                let notoriety = b.attr(
                    AttrDef::new(NOTORIETY)
                        .range(0, i64::MAX)
                        .default(0)
                        .summary("Criminal fame, 0 and up."),
                );
                Params {
                    $($field: b.param_id(concat!("missions.", stringify!($field))),)*
                    global_timeout: b.param_id("kernel.global_timeout"),
                    standing,
                    notoriety,
                    sponsor_faucet: b.faucet(SPONSOR_FAUCET),
                    sponsor_sink: b.sink(SPONSOR_SINK),
                    season_sink: b.sink(SEASON_SINK),
                }
            }
        }
    };
}

params!(
    title_max,
    text_max,
    objectives_max,
    match_max,
    event_kind_max,
    field_name_max,
    field_text_max,
    count_max,
    reward_min,
    reward_max,
    takers_max,
    deadline_min,
    deadline_max,
    sponsor_name_max,
    open_max,
    posted_max,
    accepted_max,
    team_members_max,
    list_max,
    sponsor_daily_limit,
    points_sponsor,
    points_resident,
    points_per_day,
    report_period,
    board_period,
    board_top,
    board_groups,
    season_first,
    season_days,
    hall_of_fame_max,
    reset_standing_keep_bp,
    reset_notoriety_keep_bp,
    reset_wealth_floor,
    reset_wealth_cut_bp,
    observe_max,
    new_minutes,
    drama_posted,
    drama_sponsored,
    drama_accepted,
    drama_completed,
    drama_closed,
    drama_season_ended,
    drama_season_started,
    drama_reset,
    drama_routine,
);

/// Returns a parameter as a count. A negative value reads as zero.
pub(crate) fn count(realm: &Realm, param: ParamId) -> usize {
    usize::try_from(realm.param(param)).unwrap_or(0)
}

/// Returns a parameter as a number of frames. A value below one reads as
/// one.
pub(crate) fn frames(realm: &Realm, param: ParamId) -> u64 {
    realm.param(param).max(1).unsigned_abs()
}

/// Returns a parameter as a number of ticks. A value outside the range of
/// `u32` reads as the nearest bound.
pub(crate) fn span(realm: &Realm, param: ParamId) -> u32 {
    u32::try_from(realm.param(param).max(0)).unwrap_or(u32::MAX)
}

/// Returns a parameter as a drama score, 0 to 100.
pub(crate) fn drama(realm: &Realm, param: ParamId) -> u8 {
    u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(100)
}
