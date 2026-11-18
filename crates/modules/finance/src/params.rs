//! The parameters of the finance module. The numbers live in
//! `data/balance.toml`; this file only looks up their ids.

use unsup_kernel::{Builder, FaucetId, ParamId, SinkId};

/// The data file of the module.
const DATA: &str = include_str!("../data/balance.toml");

/// Ids of every parameter, faucet and sink that the module uses.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    /// `kernel.global_timeout`: how long a verb waits for the global realm.
    pub(crate) global_timeout: ParamId,

    pub(crate) account_open_fee: ParamId,
    pub(crate) withdraw_fee_bp: ParamId,
    pub(crate) transfer_fee_bp: ParamId,
    pub(crate) deposit_spread_bp: ParamId,

    pub(crate) base_rate_bp: ParamId,
    pub(crate) loan_spread_bp: ParamId,
    pub(crate) reserve_ratio_bp: ParamId,
    pub(crate) loan_min: ParamId,
    pub(crate) loan_limit_max: ParamId,
    pub(crate) loan_base_limit: ParamId,
    pub(crate) loan_limit_per_account_day: ParamId,
    pub(crate) loan_limit_per_repaid_loan: ParamId,
    pub(crate) loan_deposit_factor_bp: ParamId,
    pub(crate) loan_collateral_bp: ParamId,
    pub(crate) loan_default_penalty_bp: ParamId,
    pub(crate) loan_term_min_days: ParamId,
    pub(crate) loan_term_max_days: ParamId,
    pub(crate) loan_first_due_days: ParamId,
    pub(crate) loan_missed_max: ParamId,
    pub(crate) bankruptcy_days: ParamId,

    pub(crate) exchange_fee_bp: ParamId,
    pub(crate) exchange_ticks: ParamId,
    pub(crate) orders_max_per_trader: ParamId,
    pub(crate) order_qty_max: ParamId,
    pub(crate) order_price_max: ParamId,
    pub(crate) book_depth: ParamId,

    pub(crate) shares_max_per_org: ParamId,
    pub(crate) share_price_max: ParamId,
    pub(crate) share_orders_max: ParamId,

    pub(crate) base_rate_floor_bp: ParamId,
    pub(crate) base_rate_ceiling_bp: ParamId,
    pub(crate) issue_limit_per_year: ParamId,
    pub(crate) retire_limit_per_year: ParamId,

    pub(crate) stats_report_period: ParamId,
    pub(crate) observe_list_max: ParamId,

    pub(crate) drama_routine: ParamId,
    pub(crate) drama_loan_granted: ParamId,
    pub(crate) drama_loan_defaulted: ParamId,
    pub(crate) drama_shares: ParamId,
    pub(crate) drama_central_bank: ParamId,

    /// Faucet `bank.loan_issue`: the bank grants a loan.
    pub(crate) loan_issue: FaucetId,
    /// Sink `bank.loan_repay`: a borrower repays principal.
    pub(crate) loan_repay: SinkId,
    /// Faucet `central_bank.issue`: money for the nation treasury.
    pub(crate) central_issue: FaucetId,
    /// Sink `central_bank.retire`: treasury money that is removed.
    pub(crate) central_retire: SinkId,
}

impl Params {
    /// Loads the data file and looks up every id. The kernel calls the
    /// plugin once per scope; definitions keep their first id.
    pub(crate) fn load(b: &mut Builder) -> Params {
        b.load_data(DATA);
        let loan_issue = b.faucet("bank.loan_issue");
        let loan_repay = b.sink("bank.loan_repay");
        let central_issue = b.faucet("central_bank.issue");
        let central_retire = b.sink("central_bank.retire");
        let id = |name: &str| b.param_id(&format!("finance.{name}"));
        Params {
            global_timeout: b.param_id("kernel.global_timeout"),
            account_open_fee: id("account_open_fee"),
            withdraw_fee_bp: id("withdraw_fee_bp"),
            transfer_fee_bp: id("transfer_fee_bp"),
            deposit_spread_bp: id("deposit_spread_bp"),
            base_rate_bp: id("base_rate_bp"),
            loan_spread_bp: id("loan_spread_bp"),
            reserve_ratio_bp: id("reserve_ratio_bp"),
            loan_min: id("loan_min"),
            loan_limit_max: id("loan_limit_max"),
            loan_base_limit: id("loan_base_limit"),
            loan_limit_per_account_day: id("loan_limit_per_account_day"),
            loan_limit_per_repaid_loan: id("loan_limit_per_repaid_loan"),
            loan_deposit_factor_bp: id("loan_deposit_factor_bp"),
            loan_collateral_bp: id("loan_collateral_bp"),
            loan_default_penalty_bp: id("loan_default_penalty_bp"),
            loan_term_min_days: id("loan_term_min_days"),
            loan_term_max_days: id("loan_term_max_days"),
            loan_first_due_days: id("loan_first_due_days"),
            loan_missed_max: id("loan_missed_max"),
            bankruptcy_days: id("bankruptcy_days"),
            exchange_fee_bp: id("exchange_fee_bp"),
            exchange_ticks: id("exchange_ticks"),
            orders_max_per_trader: id("orders_max_per_trader"),
            order_qty_max: id("order_qty_max"),
            order_price_max: id("order_price_max"),
            book_depth: id("book_depth"),
            shares_max_per_org: id("shares_max_per_org"),
            share_price_max: id("share_price_max"),
            share_orders_max: id("share_orders_max"),
            base_rate_floor_bp: id("base_rate_floor_bp"),
            base_rate_ceiling_bp: id("base_rate_ceiling_bp"),
            issue_limit_per_year: id("issue_limit_per_year"),
            retire_limit_per_year: id("retire_limit_per_year"),
            stats_report_period: id("stats_report_period"),
            observe_list_max: id("observe_list_max"),
            drama_routine: id("drama_routine"),
            drama_loan_granted: id("drama_loan_granted"),
            drama_loan_defaulted: id("drama_loan_defaulted"),
            drama_shares: id("drama_shares"),
            drama_central_bank: id("drama_central_bank"),
            loan_issue,
            loan_repay,
            central_issue,
            central_retire,
        }
    }
}
