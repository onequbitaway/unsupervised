//! Jobs, firms, production, shops, property and taxes: the real economy.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`
//! for the contract, `docs/design/economy.md` for the model and
//! `docs/design/modules/economy.md` for the rules and formulas of this
//! module.
//!
//! # Map of the crate
//!
//! - `data/` holds every number: parameters, goods, recipes, site kinds and
//!   depot prices.
//! - `rules` and `ids` read the data when the world is made.
//! - `state` holds what the module remembers: jobs, prices, leases and
//!   nodes of a zone, and the ties of a resident.
//! - `verbs` holds one file per group of verbs.
//! - `calc` holds the formulas. `labour`, `hiring` and `ownership` hold
//!   what several verbs share. `observe` builds the observation sections.
//! - `public_works`, `tenancy`, `daily`, `board` and `estate` hold the
//!   parts that work across realms or run on a schedule.
//! - [`events`] names the events and [`site`] the site tags and attributes
//!   that other modules can read.

use std::sync::Arc;

use unsup_kernel::{Builder, NoArgs, Plugin, Scope, Stage};

mod board;
mod calc;
mod daily;
mod estate;
pub mod events;
mod hiring;
mod ids;
mod labour;
mod observe;
mod ownership;
mod public_works;
mod rules;
pub mod site;
mod state;
mod tenancy;
mod verbs;

pub use board::{ListJobsArgs, ListPropertyArgs, Offer};
pub use daily::NOTICE_TAX_DUE;
pub use estate::{DESPAWNED, LOAN_DEFAULTED, RESIDENT_DIED};
pub use hiring::NOTICE_JOB_ENDED;
pub use ids::{
    EXPORT_SALE, FAUCET_HARVEST, FAUCET_OUTPUT, FAUCET_PUBLIC_WORKS, IMPORT_PURCHASE, SINK_INPUT,
};
pub use ownership::NOTICE_PROPERTY_LOST;
pub use tenancy::{NOTICE_EVICTED, NOTICE_LEASE_ENDED, NOTICE_RENT_MISSED};
pub use verbs::business::OpenBusinessArgs;
pub use verbs::gather::GatherArgs;
pub use verbs::jobs::{FireArgs, JobArgs, NOTICE_WORKER_HIRED, PostJobArgs};
pub use verbs::lease::{EndLeaseArgs, NOTICE_TENANT_MOVED_IN, RentArgs, RentOutArgs};
pub use verbs::produce::{ListRecipesArgs, ProduceArgs};
pub use verbs::property::{BuyPropertyArgs, NOTICE_PROPERTY_SOLD, SellPropertyArgs};
pub use verbs::shop::{BuyArgs, SellArgs, SetPriceArgs, StockArgs};
pub use verbs::till::{TillCollectArgs, TillFundArgs};

use ids::{Econ, Ids};
use verbs::{query, spec, timed};

/// The economy module.
#[derive(Clone, Copy, Debug, Default)]
pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn name(&self) -> &'static str {
        verbs::MODULE
    }

    fn build(&self, b: &mut Builder) {
        let (ids, rules) = Ids::register(b);
        let econ = Arc::new(Econ { ids, rules });
        b.component::<state::ties::Ties>();
        match b.scope() {
            Scope::Zone => build_zone(b, &econ),
            Scope::Global => build_global(b, &econ),
        }
        build_work_verbs(b, &econ);
        build_trade_verbs(b, &econ);
        build_property_verbs(b, &econ);
    }
}

/// Registers the state, the systems and the handlers of a zone.
fn build_zone(b: &mut Builder, econ: &Arc<Econ>) {
    b.resource::<state::jobs::JobBook>();
    b.resource::<state::prices::PriceBook>();
    b.resource::<state::leases::Leases>();
    b.resource::<state::nodes::Nodes>();
    b.resource::<board::BoardCache>();
    b.resource::<public_works::PublicWorks>();
    b.resource::<daily::Calendar>();

    // The sites of world generation go on the boards in the first tick.
    b.on_worldgen(1_000, |realm, _| board::touch(realm));

    let e = Arc::clone(econ);
    b.system(Stage::Pre, 0, "economy.daily", move |realm| daily::run(&e, realm));
    let e = Arc::clone(econ);
    b.system(Stage::Post, 100, "economy.board", move |realm| board::sync(&e, realm));

    b.on_resident_message::<state::ties::TieChange>(|realm, idx, _, change| {
        state::ties::on_change(realm, idx, change);
    });
    let e = Arc::clone(econ);
    b.on_resident_message::<tenancy::RentDue>(move |realm, idx, _, due| {
        tenancy::on_rent_due(&e, realm, idx, &due);
    });
    let e = Arc::clone(econ);
    b.on_message::<tenancy::RentAnswer>(move |realm, _, answer| {
        tenancy::on_rent_answer(&e, realm, &answer);
    });
    b.on_message::<verbs::jobs::JobLeave>(|realm, _, leave| {
        verbs::jobs::on_job_leave(realm, &leave)
    });
    b.on_message::<public_works::Answer>(|realm, _, answer| public_works::on_answer(realm, answer));
    b.on_message::<public_works::BudgetState>(|realm, _, news| {
        public_works::on_budget_state(realm, &news);
    });
    b.on_message::<estate::Foreclose>(|realm, _, order| estate::on_foreclose(realm, &order));
    b.on_message::<estate::Estate>(|realm, _, estate| estate::on_estate(realm, &estate));
    b.on_event("kernel.action_interrupted", public_works::on_interrupted);
    b.on_depart(public_works::on_depart);

    let e = Arc::clone(econ);
    b.observe("job", 0, move |realm, idx, opts| observe::job(&e, realm, idx, opts));
    let e = Arc::clone(econ);
    b.observe("business", 0, move |realm, idx, opts| observe::business(&e, realm, idx, opts));
    let e = Arc::clone(econ);
    b.observe("property", 0, move |realm, idx, opts| observe::property(&e, realm, idx, opts));
}

/// Registers the state and the handlers of the global realm.
fn build_global(b: &mut Builder, econ: &Arc<Econ>) {
    b.resource::<board::Boards>();
    b.resource::<public_works::PublicBudget>();
    b.resource::<estate::Departed>();

    b.on_message::<board::BoardUpdate>(|realm, _, update| board::on_update(realm, update));
    let e = Arc::clone(econ);
    b.on_message::<public_works::Claim>(move |realm, from, claim| {
        public_works::on_claim(&e, realm, from, &claim);
    });
    let e = Arc::clone(econ);
    b.on_message::<public_works::Release>(move |realm, _, release| {
        public_works::on_release(&e, realm, &release);
    });
    b.on_event(estate::LOAN_DEFAULTED, estate::on_loan_defaulted);
    b.on_event(estate::RESIDENT_DIED, estate::on_resident_died);
    b.on_event(estate::DESPAWNED, estate::on_despawned);
    b.system(Stage::Post, 0, "economy.departed", estate::settle_departed);
}

/// Registers the verbs of work: jobs, shifts, gathering and production.
fn build_work_verbs(b: &mut Builder, econ: &Arc<Econ>) {
    let ids = econ.ids;

    let e = Arc::clone(econ);
    b.global_verb::<ListJobsArgs>(
        query("list_jobs", "Read the job board of your city: jobs that take workers."),
        move |realm, ctx, args| board::list_jobs(&e, realm, ctx, args),
    );
    let e = Arc::clone(econ);
    timed::<PostJobArgs>(
        b,
        spec("post_job", "Offer a job at the site where you are. You need to control the site."),
        ids.paperwork_minutes,
        move |realm, ctx, args, phase| verbs::jobs::post_job(&e, realm, ctx, args, phase),
    );
    timed::<JobArgs>(
        b,
        spec("close_job", "End a job at the site where you are. Its workers lose the job."),
        ids.paperwork_minutes,
        verbs::jobs::close_job,
    );
    let e = Arc::clone(econ);
    timed::<JobArgs>(
        b,
        spec("apply_job", "Take a job at the site where you are. You can have one job."),
        ids.paperwork_minutes,
        move |realm, ctx, args, phase| verbs::jobs::apply_job(&e, realm, ctx, args, phase),
    );
    timed::<NoArgs>(
        b,
        spec("quit_job", "Leave your job. It works from every place."),
        ids.paperwork_minutes,
        |realm, ctx, _, phase| verbs::jobs::quit_job(realm, ctx, phase),
    );
    timed::<FireArgs>(
        b,
        spec("fire", "End the job of a worker at the site where you are."),
        ids.paperwork_minutes,
        verbs::jobs::fire,
    );
    let (begin, finish) = (Arc::clone(econ), Arc::clone(econ));
    b.verb::<NoArgs>(
        spec(
            "work",
            "Work one shift at the site where you are: your job there, or public works at a city hall or a depot. The wage comes at the end.",
        ),
        move |realm, ctx, _| verbs::work::begin(&begin, realm, ctx),
        move |realm, ctx, _| verbs::work::finish(&finish, realm, ctx),
    );
    let (begin, finish) = (Arc::clone(econ), Arc::clone(econ));
    b.verb::<GatherArgs>(
        spec("gather", "Gather raw goods at a farm, a mine or a forest. The goods go to you."),
        move |realm, ctx, args| verbs::gather::begin(&begin, realm, ctx, args),
        move |realm, ctx, args| verbs::gather::finish(&finish, realm, ctx, args),
    );
    let (begin, finish) = (Arc::clone(econ), Arc::clone(econ));
    b.verb::<ProduceArgs>(
        spec(
            "produce",
            "Run a recipe at a workshop, a factory or a restaurant. It uses the stock of the site and its outputs go into that stock.",
        ),
        move |realm, ctx, args| verbs::produce::begin(&begin, realm, ctx, args),
        move |realm, ctx, args| verbs::produce::finish(&finish, realm, ctx, args),
    );
    let e = Arc::clone(econ);
    b.instant_verb::<ListRecipesArgs>(
        query("list_recipes", "Read the recipes: inputs, outputs, minutes, skill and site kinds."),
        move |realm, _, args| verbs::produce::list_recipes(&e, realm, args),
    );
}

/// Registers the verbs of trade: stock, prices, sales and the till.
fn build_trade_verbs(b: &mut Builder, econ: &Arc<Econ>) {
    let ids = econ.ids;

    let e = Arc::clone(econ);
    timed::<StockArgs>(
        b,
        spec("stock", "Move goods that you carry into the stock of the site where you are."),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::shop::stock(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<StockArgs>(
        b,
        spec("unstock", "Take goods out of the stock of a site that you control."),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::shop::unstock(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<SetPriceArgs>(
        b,
        spec(
            "set_price",
            "Set what your shop or restaurant takes for a good, and what it pays to residents that sell the good to it.",
        ),
        ids.paperwork_minutes,
        move |realm, ctx, args, phase| verbs::shop::set_price(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<BuyArgs>(
        b,
        spec("buy", "Buy goods with cash at the shop, the restaurant or the depot where you are."),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::shop::buy(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<SellArgs>(
        b,
        spec(
            "sell",
            "Sell goods that you carry to the shop, the restaurant or the depot where you are.",
        ),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::shop::sell(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<TillCollectArgs>(
        b,
        spec(
            "till_collect",
            "Take cash out of the till of a site that you control. The cash of a site of an organization goes to its treasury.",
        ),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::till::till_collect(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<TillFundArgs>(
        b,
        spec(
            "till_fund",
            "Put your cash into the till of a site that you control, for wages and property tax.",
        ),
        ids.trade_minutes,
        move |realm, ctx, args, phase| verbs::till::till_fund(&e, realm, ctx, args, phase),
    );
}

/// Registers the verbs of property: sales, leases and permits.
fn build_property_verbs(b: &mut Builder, econ: &Arc<Econ>) {
    let ids = econ.ids;

    let e = Arc::clone(econ);
    b.global_verb::<ListPropertyArgs>(
        query("list_property", "Read the property board of your city: sites for sale or to let."),
        move |realm, ctx, args| board::list_property(&e, realm, ctx, args),
    );
    timed::<BuyPropertyArgs>(
        b,
        spec("buy_property", "Buy the site where you are with cash, when it is for sale."),
        ids.deal_minutes,
        verbs::property::buy_property,
    );
    let e = Arc::clone(econ);
    timed::<SellPropertyArgs>(
        b,
        spec(
            "sell_property",
            "Offer a site that you control for sale at a price, or end the offer.",
        ),
        ids.paperwork_minutes,
        move |realm, ctx, args, phase| verbs::property::sell_property(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<RentOutArgs>(
        b,
        spec(
            "rent_out",
            "Offer a home that you control to tenants at a daily rent, or end the offer.",
        ),
        ids.paperwork_minutes,
        move |realm, ctx, args, phase| verbs::lease::rent_out(&e, realm, ctx, args, phase),
    );
    let e = Arc::clone(econ);
    timed::<RentArgs>(
        b,
        spec(
            "rent",
            "Become a tenant of the home where you are. You pay the rent each day in cash.",
        ),
        ids.deal_minutes,
        move |realm, ctx, args, phase| verbs::lease::rent(&e, realm, ctx, args, phase),
    );
    timed::<EndLeaseArgs>(
        b,
        spec(
            "end_lease",
            "End your lease of the home where you are, or the lease of a tenant of your site.",
        ),
        ids.paperwork_minutes,
        verbs::lease::end_lease,
    );
    let e = Arc::clone(econ);
    timed::<OpenBusinessArgs>(
        b,
        spec(
            "open_business",
            "Pay the permit fee and open a site that you control as a shop, a restaurant, a workshop, a factory, an office or a warehouse.",
        ),
        ids.deal_minutes,
        move |realm, ctx, args, phase| verbs::business::open_business(&e, realm, ctx, args, phase),
    );
    timed::<NoArgs>(
        b,
        spec(
            "close_business",
            "Give up the permit of a business that you control. Its prices and jobs end.",
        ),
        ids.deal_minutes,
        verbs::business::close_business,
    );
}

/// Returns the plugin of the economy module.
#[must_use]
pub fn plugin() -> EconomyPlugin {
    EconomyPlugin
}
