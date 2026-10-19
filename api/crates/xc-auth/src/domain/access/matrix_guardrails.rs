//! RBAC §11.3: the guardrails every save of a group's set passes (V.9 :59-66, mapped).
//!
//! Skeleton (claude/65357afb, `feat/rbac-groups`): the body is C6b's.
use serde::{Deserialize, Serialize};

use super::permission::Permission;
use super::role_grants::Group;

/// The guardrail a set breaks. The number is the `n` of F12's `409 guardrail:<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailBreach {
    /// 1. `AUDIT_LOGS` above VIEW.
    AuditLogsAboveView,
    /// 2. `USER_MANAGEMENT` or `SYSTEM_SETTINGS` above VIEW for a group whose base is neither EXECUTIVE nor ADMIN.
    AdminModulesAboveView,
    /// 3. A group whose base is ADMIN above VIEW outside `USER_MANAGEMENT` and `SYSTEM_SETTINGS`.
    AdminBaseAboveView,
    /// 4. The built-in EXECUTIVE group: it is fixed.
    ExecutiveFixed,
    /// 5. The set's no-module permissions differ from its base's seed (they are not edited in the matrix).
    NoModuleEdited,
}

impl GuardrailBreach {
    /// 1 to 5, as §11.3 numbers them.
    pub fn number(self) -> u8 {
        match self {
            GuardrailBreach::AuditLogsAboveView => 1,
            GuardrailBreach::AdminModulesAboveView => 2,
            GuardrailBreach::AdminBaseAboveView => 3,
            GuardrailBreach::ExecutiveFixed => 4,
            GuardrailBreach::NoModuleEdited => 5,
        }
    }
}

pub struct MatrixGuardrails;

impl MatrixGuardrails {
    /// May `group` hold exactly `set`? Rule 4 is checked first, then 1, 2, 3 and 5; the first broken one is answered.
    pub fn check(group: &Group, set: &[Permission]) -> Result<(), GuardrailBreach> {
        let _ = (group, set);
        todo!("C6b")
    }
}
