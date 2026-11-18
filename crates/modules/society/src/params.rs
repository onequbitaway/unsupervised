//! The parameters of the module. Every number comes from
//! `data/balance.toml`.

use unsup_kernel::{Builder, ParamId, Realm};

/// The data file of the module.
const DATA: &str = include_str!("../data/balance.toml");

macro_rules! params {
    ($($field:ident),* $(,)?) => {
        /// The ids of the parameters of the module. The name of a field is
        /// the name of the parameter without the prefix `society.`.
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
                    $($field: b.param_id(concat!("society.", stringify!($field))),)*
                    global_timeout: b.param_id("kernel.global_timeout"),
                }
            }
        }
    };
}

params!(
    say_radius,
    say_text_max,
    says_per_hour,
    message_text_max,
    messages_per_hour,
    org_says_per_hour,
    org_broadcasts_per_hour,
    post_text_max,
    post_ticks,
    posts_per_day,
    org_create_fee,
    org_name_max,
    role_name_max,
    org_roles_max,
    org_members_max,
    org_invites_max,
    org_invite_ttl,
    memberships_max,
    contract_clauses_max,
    contracts_open_max,
    contract_due_max,
    contract_offer_ttl,
    contract_offer_ttl_max,
    contract_match_max,
    contract_event_kind_max,
    contract_field_name_max,
    contract_field_text_max,
    contract_handover_radius,
    relations_max,
    proposal_ttl,
    proposals_max,
    household_members_max,
    rep_contract_fulfilled,
    rep_contract_breached,
    rep_contract_rewards_per_day,
    opinion_weight,
    opinions_per_day,
    opinion_min_age,
    brief_list_max,
    drama_said,
    drama_posted,
    drama_org_created,
    drama_org_joined,
    drama_org_left,
    drama_org_dissolved,
    drama_org_granted,
    drama_treasury,
    drama_contract_signed,
    drama_contract_fulfilled,
    drama_contract_breached,
    drama_contract_expired,
    drama_married,
    drama_divorced,
    drama_household,
    drama_opinion,
);

/// Returns a parameter as a count. A negative value reads as zero.
pub(crate) fn count(realm: &Realm, param: ParamId) -> usize {
    usize::try_from(realm.param(param)).unwrap_or(0)
}

/// Returns a parameter as a number of ticks or tiles. A value outside the
/// range of `u32` reads as the nearest bound.
pub(crate) fn span(realm: &Realm, param: ParamId) -> u32 {
    u32::try_from(realm.param(param).max(0)).unwrap_or(u32::MAX)
}

/// Returns a parameter as a drama score, 0 to 100.
pub(crate) fn drama(realm: &Realm, param: ParamId) -> u8 {
    u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(100)
}
