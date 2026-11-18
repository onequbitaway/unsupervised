//! The ids of everything that the module registers: parameters, attributes,
//! faucets and sinks.

use unsup_kernel::{AttrDef, AttrId, Builder, FaucetId, ParamId, SinkId};

use crate::rules::Rules;

/// Name of the money faucet for starter shifts.
pub const FAUCET_PUBLIC_WORKS: &str = "gov.public_works";
/// Name of the money faucet for goods that the depot buys, and of the goods
/// sink for the same trade.
pub const EXPORT_SALE: &str = "export.sale";
/// Name of the money sink for goods that the depot sells, and of the goods
/// faucet for the same trade.
pub const IMPORT_PURCHASE: &str = "import.purchase";
/// Name of the goods faucet of `gather`.
pub const FAUCET_HARVEST: &str = "nature.harvest";
/// Name of the goods faucet of recipes.
pub const FAUCET_OUTPUT: &str = "production.output";
/// Name of the goods sink of recipes.
pub const SINK_INPUT: &str = "production.input";

/// The ids that the handlers of the module use.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ids {
    pub(crate) shift_minutes: ParamId,
    pub(crate) min_wage: ParamId,
    pub(crate) wage_max: ParamId,
    pub(crate) job_slots_max: ParamId,
    pub(crate) site_jobs_max: ParamId,
    pub(crate) income_tax_bp: ParamId,
    pub(crate) xp_per_hour: ParamId,
    pub(crate) xp_per_sale: ParamId,
    pub(crate) mood_floor_bp: ParamId,
    pub(crate) energy_per_hour: ParamId,
    pub(crate) labor_min_energy: ParamId,
    pub(crate) skill_bonus_bp: ParamId,
    pub(crate) speed_bonus_bp: ParamId,
    pub(crate) gather_minutes: ParamId,
    pub(crate) gather_minutes_min: ParamId,
    pub(crate) gather_minutes_max: ParamId,
    pub(crate) produce_batches_max: ParamId,
    pub(crate) public_works_wage: ParamId,
    pub(crate) public_works_budget: ParamId,
    pub(crate) trade_minutes: ParamId,
    pub(crate) paperwork_minutes: ParamId,
    pub(crate) deal_minutes: ParamId,
    pub(crate) sales_tax_bp: ParamId,
    pub(crate) trade_qty_max: ParamId,
    pub(crate) price_max: ParamId,
    pub(crate) property_tax_bp: ParamId,
    pub(crate) property_tax_grace_days: ParamId,
    pub(crate) city_rent_bp: ParamId,
    pub(crate) rent_grace_days: ParamId,
    pub(crate) permit_fee: ParamId,
    pub(crate) daily_spread_minutes: ParamId,
    pub(crate) list_default: ParamId,
    pub(crate) list_max: ParamId,
    pub(crate) board_refresh_minutes: ParamId,
    pub(crate) observe_prices_max: ParamId,
    pub(crate) observe_jobs_max: ParamId,
    pub(crate) observe_sites_max: ParamId,

    /// `need.mood`: it scales the harvest and the experience.
    pub(crate) mood: AttrId,
    /// `need.energy`: work costs energy.
    pub(crate) energy: AttrId,
    /// `skill.trading`: a sale trains it.
    pub(crate) trading: AttrId,

    /// Money for starter shifts.
    pub(crate) public_works: FaucetId,
    /// Money for goods that the depot buys.
    pub(crate) export_money: FaucetId,
    /// Goods that the depot sells.
    pub(crate) import_goods: FaucetId,
    /// Goods from `gather`.
    pub(crate) harvest: FaucetId,
    /// Goods from recipes.
    pub(crate) output: FaucetId,
    /// Money that the depot takes for goods.
    pub(crate) import_money: SinkId,
    /// Goods that the depot buys.
    pub(crate) export_goods: SinkId,
    /// Goods that recipes use.
    pub(crate) input: SinkId,
}

impl Ids {
    /// Registers the data of the module and returns the ids. The kernel
    /// calls it in both passes; every name keeps its first definition.
    pub(crate) fn register(b: &mut Builder) -> (Ids, Rules) {
        b.load_data(include_str!("../data/balance.toml"));
        b.load_data(include_str!("../data/items.toml"));
        crate::events::check_drama_params(b);
        let rules = Rules::load(b);
        // The life module owns the needs. The definitions are those of
        // `docs/design/game-design.md`, section 3.
        let mood = b.attr(
            AttrDef::new("need.mood")
                .range(0, 1_000)
                .default(1_000)
                .summary("Mood. It changes work output and learning."),
        );
        let energy = b.attr(
            AttrDef::new("need.energy")
                .range(0, 1_000)
                .default(1_000)
                .summary("Energy. 0 means exhausted."),
        );
        let ids = Ids {
            shift_minutes: b.param_id("economy.shift_minutes"),
            min_wage: b.param_id("economy.min_wage"),
            wage_max: b.param_id("economy.wage_max"),
            job_slots_max: b.param_id("economy.job_slots_max"),
            site_jobs_max: b.param_id("economy.site_jobs_max"),
            income_tax_bp: b.param_id("economy.income_tax_bp"),
            xp_per_hour: b.param_id("economy.xp_per_hour"),
            xp_per_sale: b.param_id("economy.xp_per_sale"),
            mood_floor_bp: b.param_id("economy.mood_floor_bp"),
            energy_per_hour: b.param_id("economy.energy_per_hour"),
            labor_min_energy: b.param_id("economy.labor_min_energy"),
            skill_bonus_bp: b.param_id("economy.skill_bonus_bp"),
            speed_bonus_bp: b.param_id("economy.speed_bonus_bp"),
            gather_minutes: b.param_id("economy.gather_minutes"),
            gather_minutes_min: b.param_id("economy.gather_minutes_min"),
            gather_minutes_max: b.param_id("economy.gather_minutes_max"),
            produce_batches_max: b.param_id("economy.produce_batches_max"),
            public_works_wage: b.param_id("economy.public_works_wage"),
            public_works_budget: b.param_id("economy.public_works_budget"),
            trade_minutes: b.param_id("economy.trade_minutes"),
            paperwork_minutes: b.param_id("economy.paperwork_minutes"),
            deal_minutes: b.param_id("economy.deal_minutes"),
            sales_tax_bp: b.param_id("economy.sales_tax_bp"),
            trade_qty_max: b.param_id("economy.trade_qty_max"),
            price_max: b.param_id("economy.price_max"),
            property_tax_bp: b.param_id("economy.property_tax_bp"),
            property_tax_grace_days: b.param_id("economy.property_tax_grace_days"),
            city_rent_bp: b.param_id("economy.city_rent_bp"),
            rent_grace_days: b.param_id("economy.rent_grace_days"),
            permit_fee: b.param_id("economy.permit_fee"),
            daily_spread_minutes: b.param_id("economy.daily_spread_minutes"),
            list_default: b.param_id("economy.list_default"),
            list_max: b.param_id("economy.list_max"),
            board_refresh_minutes: b.param_id("economy.board_refresh_minutes"),
            observe_prices_max: b.param_id("economy.observe_prices_max"),
            observe_jobs_max: b.param_id("economy.observe_jobs_max"),
            observe_sites_max: b.param_id("economy.observe_sites_max"),
            mood,
            energy,
            trading: b.attr_id("skill.trading"),
            public_works: b.faucet(FAUCET_PUBLIC_WORKS),
            export_money: b.faucet(EXPORT_SALE),
            import_goods: b.faucet(IMPORT_PURCHASE),
            harvest: b.faucet(FAUCET_HARVEST),
            output: b.faucet(FAUCET_OUTPUT),
            import_money: b.sink(IMPORT_PURCHASE),
            export_goods: b.sink(EXPORT_SALE),
            input: b.sink(SINK_INPUT),
        };
        (ids, rules)
    }
}

/// The ids and the rules of the module. Every handler shares one.
#[derive(Debug)]
pub(crate) struct Econ {
    /// Ids of parameters, attributes, faucets and sinks.
    pub(crate) ids: Ids,
    /// What the data files say.
    pub(crate) rules: Rules,
}
