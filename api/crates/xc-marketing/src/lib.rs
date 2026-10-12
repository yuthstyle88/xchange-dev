//! `xc-marketing` — campaigns, benefit rules, coupons, budgets, segments (design
//! `docs/design/blueprint/marketing-campaigns.md`, #560, D-1). `xc-exchange` never depends on it.
//!
//! SCAFFOLD (#1390 up-front tests): the items the provided tests call, with the shapes they use.
//! Every body is `unimplemented!()`; MK-1 writes them.

pub mod domain {
    pub mod campaign;
}

pub mod infrastructure {
    pub mod pg_campaign_repository;
}
