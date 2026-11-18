//! Crime, police, courts and jail.
//!
//! This crate is a plugin for the kernel. A resident can steal, burgle,
//! rob, attack, vandalize, bribe and carry contraband. The world produces
//! evidence. Police, who are residents with permissions, investigate and
//! arrest. Courts decide. The engine carries out every penalty by itself,
//! and no step waits without a limit for a resident.
//!
//! The rules and the formulas are in `docs/design/modules/crime.md`. The
//! numbers are in `data/balance.toml`.
//!
//! # Map of the crate
//!
//! - `verbs`: the crimes (`pickpocket`, `burgle`, `rob`, `assault`,
//!   `vandalize`), and `repair`, `fence`, `bribe`, `report_crime` and
//!   `pay_fine`.
//! - `commit`: the common end of every crime: the case, the trace, the
//!   witnesses, the city watch and the public event.
//! - `record`, `hot`, `loot`, `seize`: the criminal record of a resident
//!   and the stolen value that it holds.
//! - `police`: `patrol`, `investigate`, `search`, `arrest` and `release`.
//! - `court`: the case registry of the global realm, trials, juries,
//!   bounties, and the verbs `charge`, `rule_case`, `jury_vote`,
//!   `post_bounty`, `case_list` and `case_view`.
//! - `enforce`, `custody`, `jail`, `debts`: the zone carries out what the
//!   registry decides.
//! - `watch`: the baseline enforcement of the engine for a world with no
//!   police.
//! - `places`, `scene`: the law sites of the world and the traces of
//!   crimes.
//!
//! # Example
//!
//! ```
//! use unsup_kernel::testing::TestWorld;
//!
//! let life = unsup_life::plugin();
//! let crime = unsup_crime::plugin();
//! let mut world = TestWorld::new(&[&life, &crime]);
//! let ada = world.spawn("Ada");
//! // Nobody is under arrest in a new world.
//! assert!(!world.has_condition(ada, "detained"));
//! assert_eq!(world.attr(ada, "rep.notoriety"), 0);
//! // An arrest needs a power of the law.
//! let bo = world.spawn("Bo");
//! let error = world.act_err(ada, "arrest", serde_json::json!({ "target": bo }));
//! assert_eq!(error.code, unsup_kernel::ErrorCode::NotAllowed);
//! ```

mod calc;
mod commit;
mod court;
mod custody;
mod debts;
mod enforce;
mod hot;
mod ids;
mod jail;
mod kinds;
mod loot;
mod msg;
pub mod names;
mod observe;
mod places;
mod police;
mod powers;
mod record;
mod remote;
mod scene;
mod seize;
mod verbs;
mod watch;

use unsup_kernel::{Builder, Kin, Plugin, Scope};

pub use crate::court::bounty::PostBountyArgs;
pub use crate::court::docket::{CaseListArgs, CaseViewArgs};
pub use crate::court::registry::{Bounty, Case, Cases, Decider, Evidence, Sentence, Status, Trial};
pub use crate::court::verbs::{ChargeArgs, JuryVoteArgs, RuleCaseArgs};
pub use crate::kinds::CrimeKind;
pub use crate::msg::Source;
pub use crate::places::{LawKind, LawSite, LawSites};
pub use crate::police::arrest::ArrestArgs;
pub use crate::police::investigate::InvestigateArgs;
pub use crate::police::patrol::PatrolArgs;
pub use crate::police::release::ReleaseArgs;
pub use crate::police::search::SearchArgs;
pub use crate::record::{Custody, Debt, Docket, HotLot, Known, Record, Role, Warrant};
pub use crate::scene::Scenes;
pub use crate::verbs::assault::AssaultArgs;
pub use crate::verbs::bribe::BribeArgs;
pub use crate::verbs::burgle::BurgleArgs;
pub use crate::verbs::fence::FenceArgs;
pub use crate::verbs::fines::PayFineArgs;
pub use crate::verbs::pickpocket::PickpocketArgs;
pub use crate::verbs::report::ReportCrimeArgs;
pub use crate::verbs::rob::RobArgs;
pub use crate::verbs::vandalize::RepairArgs;

/// The crime module.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrimePlugin;

impl Plugin for CrimePlugin {
    fn name(&self) -> &'static str {
        verbs::MODULE
    }

    fn build(&self, b: &mut Builder) {
        let h = ids::Handles::register(b);
        if b.scope() == Scope::Zone {
            b.component::<record::Record>();
            // `unsup_life::die` reads these two components. A world
            // without the life module still has them.
            b.component::<Kin>();
            b.component::<unsup_life::Body>();
        }
        places::register(b);
        scene::register(b, h);
        hot::register(b, h);
        enforce::register(b, h);
        jail::register(b, h);
        watch::register(b, h);

        verbs::pickpocket::register(b, h);
        verbs::burgle::register(b, h);
        verbs::rob::register(b, h);
        verbs::assault::register(b, h);
        verbs::vandalize::register(b, h);
        verbs::fence::register(b, h);
        verbs::bribe::register(b, h);
        verbs::report::register(b);
        verbs::fines::register(b, h);

        police::patrol::register(b, h);
        police::investigate::register(b, h);
        police::search::register(b, h);
        police::arrest::register(b, h);
        police::release::register(b, h);

        court::register(b, h);
        observe::register(b, h);
    }
}

/// Returns the plugin of the crime module.
#[must_use]
pub fn plugin() -> CrimePlugin {
    CrimePlugin
}
