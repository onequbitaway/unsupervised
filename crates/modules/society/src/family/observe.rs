//! The observation section `relations`: the spouse, the household, the
//! private labels and the open proposals of a resident.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Idx, Kin, ObserveOpts, Realm, ResidentId, Value};

use crate::social::{Label, Social};

/// Builds the section, or nothing when the resident has no relation.
pub(crate) fn section(realm: &Realm, idx: Idx, _opts: &ObserveOpts) -> Option<Value> {
    let mut out = Map::new();
    {
        let kin = &realm.col::<Kin>()[idx];
        if let Some(spouse) = kin.spouse {
            out.insert("spouse".into(), json!(spouse));
        }
        if let Some(household) = kin.household {
            out.insert("household".into(), json!(household));
        }
    }
    let column = realm.col::<Social>();
    if let Some(state) = column[idx].get() {
        let mut labels = Map::new();
        for label in Label::ALL {
            let with: Vec<ResidentId> =
                state.labels.iter().filter(|(_, l)| *l == label).map(|(id, _)| *id).collect();
            if !with.is_empty() {
                labels.insert(label.name().into(), json!(with));
            }
        }
        if !labels.is_empty() {
            out.insert("labels".into(), Value::Object(labels));
        }
        let frame = realm.frame();
        let proposals: Vec<Value> = state
            .suits
            .iter()
            .filter(|suit| suit.expires >= frame)
            .map(|suit| json!({ "from": suit.from, "ask": suit.ask.name(), "expires": suit.expires }))
            .collect();
        if !proposals.is_empty() {
            out.insert("proposals".into(), json!(proposals));
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}
