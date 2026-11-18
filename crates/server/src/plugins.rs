//! The list of game modules. This is the only place that knows them all.

use unsup_kernel::Plugin;

/// Returns every game module, in the order in which the kernel builds them.
///
/// The order matters only where one module uses a name that another module
/// defines (an attribute, an item, a parameter). A module that defines a
/// name comes before a module that only looks it up.
#[must_use]
pub fn plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(unsup_life::plugin()),
        Box::new(unsup_economy::plugin()),
        Box::new(unsup_finance::plugin()),
        Box::new(unsup_society::plugin()),
        Box::new(unsup_crime::plugin()),
        Box::new(unsup_politics::plugin()),
        Box::new(unsup_missions::plugin()),
    ]
}
