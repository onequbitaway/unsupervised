//! The verb `org_create`. The zone takes the fee from the cash of the
//! founder. The global realm makes the record.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Holder, OrgId, Realm, Value};

use super::args::OrgCreateArgs;
use super::mirror::{describe_with, mirror};
use super::state::{Org, Orgs};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::{pay, perms, spec, text};

/// Ledger label of the fee for a new organization.
const FEE: &str = "org.fee";

/// The global part of `org_create`.
#[derive(Serialize, Deserialize)]
struct Create {
    name: String,
    open: bool,
}

impl Call for Create {
    const KIND: &'static str = "society.org_create";
}

fn run(realm: &Realm, p: &Params, origin: &Origin, call: Create) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    if orgs.name_taken(&call.name) {
        return Err(ActionError::conflict("An organization has this name already."));
    }
    let max = params::count(realm, p.memberships_max);
    if orgs.membership_count(origin.resident) >= max {
        return Err(ActionError::conflict(format!(
            "You are in too many organizations. The limit is {max}."
        )));
    }
    let id = OrgId(realm.alloc_id());
    // The registry keeps the permissions of a role sorted.
    let role = |perms: &[&str]| {
        let mut list: Vec<String> = perms.iter().map(|perm| (*perm).to_string()).collect();
        list.sort();
        list
    };
    orgs.insert(Org {
        id,
        name: call.name.trim().to_string(),
        founder: Some(origin.resident),
        open: call.open,
        roles: BTreeMap::from([
            (perms::OWNER_ROLE.to_string(), role(&perms::OWNER_PERMISSIONS)),
            (perms::MEMBER_ROLE.to_string(), role(&[])),
        ]),
        ..Org::default()
    });
    orgs.set_member(id, origin.resident, perms::OWNER_ROLE);
    // The fee goes to the city of the founder.
    let fee = pay::transfer(
        realm,
        origin.escrow(),
        Holder::Org(OrgId::city(origin.city)),
        origin.held,
        FEE,
    );
    let Some(org) = orgs.get(id) else {
        return Err(ActionError::internal("The new organization is not in the registry."));
    };
    mirror(realm, org, origin.resident);
    realm.emit(
        EventMeta::public("society.org_created", params::drama(realm, p.drama_org_created))
            .actor(origin.resident)
            .at(origin.pos),
        &describe_with(org, json!({ "founder": origin.resident, "open": org.open })),
    );
    Ok(json!({ "org": id, "role": perms::OWNER_ROLE, "fee": fee }))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<OrgCreateArgs, Create>(
        b,
        spec(
            "org_create",
            "Found an organization: a company, a union, a party, a gang. You become its owner. It costs a fee.",
        ),
        p.global_timeout,
        move |realm, _ctx, args| {
            text::check_text(&args.name, realm.param(p.org_name_max), "name")?;
            if args.name.contains('\n') {
                return Err(ActionError::invalid_args("The name has a line break."));
            }
            let call = Create { name: args.name.clone(), open: args.open };
            Ok((call, realm.param(p.org_create_fee)))
        },
        move |realm, origin, call| run(realm, &p, origin, call),
    );
}
