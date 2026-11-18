//! The observation section `orgs`: the organizations of a resident and its
//! open invitations.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Holder, ObserveOpts, Realm, ResidentId, Value};

use super::mirror::{describe, describe_with};
use super::state::Orgs;
use crate::params::{self, Params};

/// Builds the section, or nothing when the resident has no organization
/// and no invitation.
///
/// With brief detail each list has at most `society.brief_list_max`
/// entries, and `more_orgs` and `more_invites` count the rest. A resident
/// does not control how many organizations invite it, so the list of
/// invitations has that limit with full detail too.
pub(crate) fn section(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    opts: &ObserveOpts,
) -> Option<Value> {
    let orgs = realm.res::<Orgs>();
    let ledger = realm.ledger();
    let brief_max = params::count(realm, p.brief_list_max);
    let orgs_max = if opts.is_full() { usize::MAX } else { brief_max };
    let member_count = orgs.membership_count(resident);
    let member_of: Vec<Value> = orgs
        .of_member(resident)
        .take(orgs_max)
        .map(|org| {
            let role = org.role_of(resident).unwrap_or_default();
            let mut entry = describe(org);
            entry.insert("role".into(), json!(role));
            entry.insert("members".into(), json!(org.member_count()));
            entry.insert("treasury".into(), json!(ledger.money(Holder::Org(org.id))));
            if opts.is_full() {
                entry.insert("permissions".into(), json!(org.effective(role)));
                entry.insert("roles".into(), json!(org.roles));
                entry.insert("grants".into(), json!(org.grants));
                entry.insert("open".into(), json!(org.open));
            }
            Value::Object(entry)
        })
        .collect();
    let frame = realm.frame();
    let invite_count = orgs.invitations(resident, frame).count();
    let invites: Vec<Value> = orgs
        .invitations(resident, frame)
        .take(brief_max)
        .map(|(org, expires)| describe_with(org, json!({ "expires": expires })))
        .collect();
    if member_count == 0 && invite_count == 0 {
        return None;
    }
    let mut out = Map::new();
    if !member_of.is_empty() {
        out.insert("member_of".into(), json!(member_of));
    }
    if member_count > member_of.len() {
        out.insert("more_orgs".into(), json!(member_count - member_of.len()));
    }
    if !invites.is_empty() {
        out.insert("invites".into(), json!(invites));
    }
    if invite_count > invites.len() {
        out.insert("more_invites".into(), json!(invite_count - invites.len()));
    }
    Some(Value::Object(out))
}
