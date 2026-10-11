//! Approval subjects, one file per subject (design `docs/design/build-time-registries.md` §2).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

/// Registers one approval subject.
pub type RegisterFn = fn(&mut ApprovalRegistry, PgPool);

include!(concat!(env!("OUT_DIR"), "/approval_subjects.rs"));

/// Builds an approval registry with all generated subjects registered.
pub fn registry(pool: PgPool) -> ApprovalRegistry {
    let mut reg = ApprovalRegistry::new();
    for register in APPROVAL_SUBJECTS {
        register(&mut reg, pool.clone());
    }
    reg
}
