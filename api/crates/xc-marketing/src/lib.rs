//! `xc-marketing` — campaigns, benefit rules, coupons, budgets, segments (design
//! `docs/design/blueprint/marketing-campaigns.md`, #560, D-1). `xc-exchange` never depends on it.

pub mod domain {
    pub mod benefit;
    pub mod campaign;
    pub mod coupon;
    pub mod engine;
}

pub mod infrastructure {
    pub mod pg_budget_ledger;
    pub mod pg_campaign_repository;
}

pub mod application {
    pub mod coupon_service;
    pub mod redeemer;
}
