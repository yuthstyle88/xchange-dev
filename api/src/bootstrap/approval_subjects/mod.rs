//! Approval subjects, one file per subject (design `docs/design/build-time-registries.md` §2, `membership-points.md` §6.4 R2).

use std::sync::Arc;

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;
use xc_loyalty::application::rule_publisher::LoyaltyRuleWriter;
use xc_loyalty::infrastructure::http_rule_writer::HttpLoyaltyRuleWriter;

use crate::config::Settings;

/// Registers one approval subject.
pub type RegisterFn = fn(&mut ApprovalRegistry, PgPool);

include!(concat!(env!("OUT_DIR"), "/approval_subjects.rs"));

/// Dependencies passed to approval subjects that need more than a database pool (LY-6 #1411, R2).
#[derive(Clone)]
pub struct ApprovalSubjectDeps {
    pub pool: PgPool,
    pub loyalty: Option<Arc<dyn LoyaltyRuleWriter>>,
}

impl ApprovalSubjectDeps {
    pub fn pool_only(pool: PgPool) -> Self {
        Self {
            pool,
            loyalty: None,
        }
    }

    pub fn from_settings(pool: PgPool, settings: &Settings) -> Self {
        let loyalty = settings
            .loyalty
            .as_ref()
            .and_then(HttpLoyaltyRuleWriter::from_config)
            .map(|w| Arc::new(w) as Arc<dyn LoyaltyRuleWriter>);
        Self { pool, loyalty }
    }
}

/// Builds an approval registry with all generated subjects registered, plus any subject needing
/// `ApprovalSubjectDeps` (R2).
pub fn registry_with(deps: &ApprovalSubjectDeps) -> ApprovalRegistry {
    let mut reg = ApprovalRegistry::new();
    for register in APPROVAL_SUBJECTS {
        register(&mut reg, deps.pool.clone());
    }
    loyalty_rule_version::register_with_deps(&mut reg, deps);
    reg
}

/// Builds an approval registry with all generated subjects registered.
pub fn registry(pool: PgPool) -> ApprovalRegistry {
    registry_with(&ApprovalSubjectDeps::pool_only(pool))
}
