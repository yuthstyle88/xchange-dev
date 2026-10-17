//! Approval subject for loyalty rule versions (LY-6 #1411, design `membership-points.md` §6, §6.4 R2).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

use super::ApprovalSubjectDeps;
use crate::application::workflows::loyalty::rule_approval;

pub const TYPE: &str = "LOYALTY_RULE_VERSION";

pub fn register(registry: &mut ApprovalRegistry, _pool: PgPool) {
    rule_approval::register_with_writer(registry, None);
}

pub fn register_with_deps(registry: &mut ApprovalRegistry, deps: &ApprovalSubjectDeps) {
    rule_approval::register_with_writer(registry, deps.loyalty.clone());
}
