//! The parameters of the module. Every number comes from
//! `data/balance.toml`.

use unsup_kernel::{Builder, ParamId, Realm};

use crate::model::Gov;

/// The data file of the module.
const DATA: &str = include_str!("../data/balance.toml");

macro_rules! params {
    ($($field:ident),* $(,)?) => {
        /// The ids of the parameters of the module. The name of a field is
        /// the name of the parameter without the prefix `politics.`.
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct Params {
            $(pub(crate) $field: ParamId,)*
            /// Ticks after which an action that waits for the global realm
            /// fails. It is a parameter of the kernel.
            pub(crate) global_timeout: ParamId,
        }

        impl Params {
            /// Loads the data file and looks up every parameter.
            pub(crate) fn load(b: &mut Builder) -> Params {
                b.load_data(DATA);
                Params {
                    $($field: b.param_id(concat!("politics.", stringify!($field))),)*
                    global_timeout: b.param_id("kernel.global_timeout"),
                }
            }
        }
    };
}

params!(
    city_term_days,
    nation_term_days,
    city_first_vote_day,
    nation_first_vote_day,
    registration_days,
    campaign_days,
    polls_open_minute,
    polls_close_minute,
    council_seats,
    legislature_seats,
    police_officers_max,
    judges_max,
    candidate_fee,
    platform_max,
    candidates_max,
    win_votes_min,
    voter_min_age,
    close_margin,
    campaign_minutes,
    campaign_cost_min,
    campaign_radius_base,
    campaign_cents_per_tile,
    campaign_radius_max,
    bill_voting_minutes,
    bill_sign_minutes,
    bill_override_minutes,
    quorum_permille,
    override_num,
    override_den,
    bills_open_max,
    bill_text_max,
    register_max,
    salary_mayor,
    salary_councillor,
    salary_president,
    salary_legislator,
    salary_police_chief,
    salary_police_officer,
    salary_judge,
    salary_central_banker,
    recall_days,
    recall_signatures_min,
    recall_permille,
    publish_minutes,
    brief_list_max,
    drama_routine,
    drama_election_called,
    drama_candidate_registered,
    drama_donation,
    drama_rally,
    drama_polls_opened,
    drama_lead_changed,
    drama_election_decided,
    drama_election_close,
    drama_election_empty,
    drama_bill_proposed,
    drama_bill_voted,
    drama_bill_passed,
    drama_bill_rejected,
    drama_law_vetoed,
    drama_law_enacted,
    drama_law_override,
    drama_appointed,
    drama_dismissed,
    drama_recall_started,
    drama_recall_succeeded,
    drama_recall_failed,
    drama_salaries_unpaid,
);

/// Returns the value of a parameter for a government, in the global realm:
/// the value of the city for a city government, the national value for the
/// nation. So a city law changes the rules of that city only.
pub(crate) fn value(realm: &Realm, param: ParamId, gov: Gov) -> i64 {
    match gov {
        Gov::Nation => realm.param(param),
        Gov::City(city) => realm.param_in(param, city),
    }
}

/// Returns the value of a parameter for a government as a number that is
/// not negative: days, minutes, seats or votes.
pub(crate) fn amount(realm: &Realm, param: ParamId, gov: Gov) -> u64 {
    u64::try_from(value(realm, param, gov)).unwrap_or(0)
}

/// Returns a parameter of this realm as a count. A negative value reads as
/// zero.
pub(crate) fn count(realm: &Realm, param: ParamId) -> usize {
    usize::try_from(realm.param(param)).unwrap_or(0)
}

/// Returns a parameter of this realm as a number of ticks or tiles. A value
/// outside the range of `u32` reads as the nearest bound.
pub(crate) fn span(realm: &Realm, param: ParamId) -> u32 {
    u32::try_from(realm.param(param).max(0)).unwrap_or(u32::MAX)
}

/// Returns a parameter of this realm as a drama score, 0 to 100.
pub(crate) fn drama(realm: &Realm, param: ParamId) -> u8 {
    u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(100)
}
