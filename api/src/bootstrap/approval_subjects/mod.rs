//! Approval subjects, one file per subject (design `docs/design/build-time-registries.md` §2, `membership-points.md` §6.4 R2).

use std::sync::Arc;

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;
use xc_loyalty::application::rule_publisher::LoyaltyRuleWriter;

use crate::bootstrap::AppState;

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

    pub fn from_state(state: &AppState) -> Self {
        Self {
            pool: state.db.clone(),
            loyalty: state.loyalty_rule_writer.clone(),
        }
    }
}

/// Builds an approval registry with all generated subjects registered, plus any subject needing
/// `ApprovalSubjectDeps` (R2).
pub fn registry_with(deps: &ApprovalSubjectDeps) -> ApprovalRegistry {
    let mut reg = ApprovalRegistry::new();
    for (register, type_name) in APPROVAL_SUBJECTS.iter().zip(APPROVAL_SUBJECT_TYPES) {
        if *type_name == loyalty_rule_version::TYPE {
            loyalty_rule_version::register_with_deps(&mut reg, deps);
        } else {
            register(&mut reg, deps.pool.clone());
        }
    }
    reg
}

/// Builds an approval registry with all generated subjects registered.
pub fn registry(pool: PgPool) -> ApprovalRegistry {
    registry_with(&ApprovalSubjectDeps::pool_only(pool))
}
