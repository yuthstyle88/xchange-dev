//! Transfer kind registry and implementations.

pub mod branch;
pub mod external;
pub mod safe;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::ids::BranchId;

pub use branch::BranchTransferEnds;
pub use external::ExternalMove;
pub use safe::SafeMove;

use crate::domain::transfer::cash_transfer::{TransferError, TransferKind};
use crate::domain::transfer::ends::TransferEnds;

#[derive(Clone)]
pub struct TransferKinds {
    kinds: HashMap<TransferKind, Arc<dyn TransferEnds>>,
}

impl Default for TransferKinds {
    fn default() -> Self {
        Self::standard()
    }
}

impl TransferKinds {
    /// Constructs registry with all standard transfer kinds.
    pub fn standard() -> Self {
        let mut kinds: HashMap<TransferKind, Arc<dyn TransferEnds>> = HashMap::new();
        kinds.insert(TransferKind::Branch, Arc::new(BranchTransferEnds::new()));
        kinds.insert(
            TransferKind::DrawerToSafe,
            Arc::new(SafeMove::new(TransferKind::DrawerToSafe)),
        );
        kinds.insert(
            TransferKind::SafeToDrawer,
            Arc::new(SafeMove::new(TransferKind::SafeToDrawer)),
        );
        kinds.insert(
            TransferKind::ExternalIn,
            Arc::new(ExternalMove::new(TransferKind::ExternalIn)),
        );
        kinds.insert(
            TransferKind::ExternalOut,
            Arc::new(ExternalMove::new(TransferKind::ExternalOut)),
        );
        Self { kinds }
    }

    /// Looks up `TransferEnds` for a given kind.
    pub fn get(&self, kind: TransferKind) -> Option<Arc<dyn TransferEnds>> {
        self.kinds.get(&kind).cloned()
    }

    /// Returns list of all registered kinds, sorted by their `as_str()`.
    pub fn kinds(&self) -> Vec<TransferKind> {
        let mut list: Vec<TransferKind> = self.kinds.keys().copied().collect();
        list.sort_by_key(|k| k.as_str());
        list
    }
}

/// A kind's access rule for one step (D:§3.1, §3.2): `who` holds `permission` on `branch`, decided by
/// `Authorizer::check` from the one `RolePermissions` table, so no kind names a role.
///
/// `TransferEnds` carries no clock (D:§4.5), so a role's validity window is read at the wall clock here;
/// `TransferFlow` checks the command's own steps at the command's `now`.
pub(crate) fn require(
    who: &Actor,
    permission: Permission,
    branch: &BranchId,
    step: &str,
) -> Result<(), TransferError> {
    let target = Target::new(&who.organization_id, Some(branch.clone()), None);
    Authorizer::check(Some(who), permission, &target, Utc::now())
        .map(|_| ())
        .map_err(|refusal| {
            TransferError::Validation(format!(
                "{step} needs {permission:?} on branch {}: {}",
                branch.as_str(),
                refusal.code()
            ))
        })
}
