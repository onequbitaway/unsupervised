//! Arguments of the organization verbs. The doc comments are the schema
//! that agents read.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::{OrgId, ResidentId};

/// Arguments of `org_create`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgCreateArgs {
    /// Name of the new organization. No other organization can have it.
    pub(crate) name: String,
    /// True: every resident can join. False: a resident needs an invitation.
    #[serde(default)]
    pub(crate) open: bool,
}

/// Arguments of verbs that name only an organization.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgArgs {
    /// Id of the organization.
    pub(crate) org: OrgId,
}

/// Arguments of verbs that name an organization and a resident.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgResidentArgs {
    /// Id of the organization.
    pub(crate) org: OrgId,
    /// Id of the resident.
    pub(crate) resident: ResidentId,
}

/// Arguments of `org_define_role`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgDefineRoleArgs {
    /// Id of the organization.
    pub(crate) org: OrgId,
    /// Name of the role: lower-case letters, digits and `_`. A role with
    /// this name is replaced.
    pub(crate) role: String,
    /// The permissions of the role, for example `["org.invite",
    /// "treasury.spend"]`. A `law.*`, `gov.*` or `bank.*` permission has an
    /// effect only while the organization holds a grant for it.
    pub(crate) permissions: Vec<String>,
}

/// Arguments of `org_set_role`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgSetRoleArgs {
    /// Id of the organization.
    pub(crate) org: OrgId,
    /// The member that gets the role.
    pub(crate) resident: ResidentId,
    /// Name of a role of the organization.
    pub(crate) role: String,
}

/// Arguments of `org_grant`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgGrantArgs {
    /// Id of the government organization that gives the permission. You
    /// need `gov.appoint` there.
    pub(crate) org: OrgId,
    /// Id of the organization that gets the permission.
    pub(crate) to: OrgId,
    /// A `law.*`, `gov.*` or `bank.*` permission that the government
    /// organization holds.
    pub(crate) permission: String,
    /// True: take the grant back.
    #[serde(default)]
    pub(crate) revoke: bool,
}

/// Arguments of `org_deposit`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgDepositArgs {
    /// Id of the organization that gets the money.
    pub(crate) org: OrgId,
    /// Cents of your cash to put in its treasury.
    pub(crate) amount: i64,
}

/// Arguments of `org_pay`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgPayArgs {
    /// Id of the organization that pays. You need `treasury.spend` there.
    pub(crate) org: OrgId,
    /// Cents to pay from its treasury.
    pub(crate) amount: i64,
    /// The resident that gets the money as cash. Give this or `to_org`.
    #[serde(default)]
    pub(crate) to_resident: Option<ResidentId>,
    /// The organization whose treasury gets the money. Give this or
    /// `to_resident`.
    #[serde(default)]
    pub(crate) to_org: Option<OrgId>,
}
