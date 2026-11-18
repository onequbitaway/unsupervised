//! The verb `report_crime`: a victim or a witness tells the police what
//! it saw.
//!
//! A resident can report only a case that it knows from a notice: it was
//! the victim, it saw the crime, or it got the bribe. The testimony names
//! the perpetrator only when the resident recognized it. So nobody can
//! put the blame on a resident that it did not see.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, Dest};

use super::free_spec;
use crate::msg::Report;
use crate::record::Record;

/// Arguments of `report_crime`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReportCrimeArgs {
    /// The case to report. The notice `crime.witnessed` or
    /// `crime.bribe_received` has the number.
    pub case: u64,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder) {
    b.instant_verb::<ReportCrimeArgs>(
        free_spec(
            "report_crime",
            "Tell the police what you saw of a case that you know as a victim or a witness. It adds evidence when you recognized the perpetrator.",
        ),
        |realm, ctx, args| {
            let (role, identified) = {
                let mut records = realm.col_mut::<Record>();
                let Some(known) =
                    records[ctx.idx].known.iter_mut().find(|known| known.case == args.case)
                else {
                    return Err(ActionError::not_found(
                        "You do not know this case. You can report only a crime that you saw or that was against you.",
                    ));
                };
                if known.reported {
                    return Err(ActionError::conflict("You reported this case already."));
                }
                known.reported = true;
                (known.role, known.suspect.is_some())
            };
            realm.send(
                Dest::Global,
                &Report::Testimony {
                    case: args.case,
                    witness: ctx.resident,
                    direct: role.is_direct(),
                    identified,
                },
            );
            Ok(json!({ "case": args.case, "reported": true, "named_suspect": identified }))
        },
    );
}
