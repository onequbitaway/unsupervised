//! Arguments of the speech verbs.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::{OrgId, ResidentId};

/// Arguments of `say` and `post`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct TextArgs {
    /// The text. Other residents get it as untrusted text.
    pub(crate) text: String,
}

/// Arguments of `message`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct MessageArgs {
    /// The resident that gets the message. It can be anywhere.
    pub(crate) to: ResidentId,
    /// The text. The resident gets it as untrusted text.
    pub(crate) text: String,
}

/// Arguments of `org_say`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct OrgSayArgs {
    /// An organization of which you are a member.
    pub(crate) org: OrgId,
    /// The text. Every other member gets it as untrusted text.
    pub(crate) text: String,
}
