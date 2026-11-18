//! Family links of a resident.

use serde::{Deserialize, Serialize};
use unsup_protocol::ResidentId;

use crate::column::Component;

/// Family links of a resident. Several modules share this component: the
/// life module reads the heir at death, the society module writes the spouse
/// and the household.
///
/// A plugin that uses it registers it with `Builder::component::<Kin>()`.
/// Registration of the same component by several plugins is safe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kin {
    /// The resident that the will names. It gets the holdings at death.
    pub heir: Option<ResidentId>,
    /// The spouse. It inherits when the will names nobody.
    pub spouse: Option<ResidentId>,
    /// The household. Members of one household share a home.
    pub household: Option<u64>,
}

impl Kin {
    /// Returns who inherits: the heir of the will, else the spouse.
    #[must_use]
    pub fn successor(&self) -> Option<ResidentId> {
        self.heir.or(self.spouse)
    }
}

impl Component for Kin {
    const NAME: &'static str = "kernel.kin";
}
