//! Policy entity. OR-7.m w2 (#1027): the v2 half (`Policy::allow`, `Policy::branch` over the v2
//! `Actor` / `Role`) is gone; every route asks `Policy::need` / `Policy::authorize`.

use xc_kernel::ids::BranchId;

use crate::domain::policy::value_objects::action::Action;
use crate::domain::policy::value_objects::entry_path::EntryPath;
use crate::domain::policy::value_objects::policy_error::PolicyError;

/// Authorisation policy engine.
pub struct Policy;

// ── OR-4 (#550): the route table onto `Authorizer` ─────────────────────────────
//
// Signatures only, from the ticket author of OR-4: the bodies are OR-4's to write
// (`api/crates/xc-auth/tests/policy_*_test.rs`, design `docs/design/blueprint/org-roles-branches.md`
// §3.5, §4.2).

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::domain::access::{
    Actor as Who, Authorizer, Grant, Permission, Refusal, Role as BpRole, RoleGrants, Target,
};

/// What a (path, name) requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Nobody's role is asked (the customer kiosk).
    Public,
    /// `Authorizer::check` with this permission.
    Permission(Permission),
    /// Any of these permissions (`Need::AnyOf`).
    AnyOf(&'static [Permission]),
    /// #582's row: until #582, the custody engine asks the v2 rule [`Policy::allow`].
    Custody,
}

fn actor_single_branch(actor: &Who) -> Option<BranchId> {
    let mut branches = HashSet::new();
    for r in &actor.roles {
        if let Some(b) = &r.branch {
            branches.insert(b.clone());
        }
    }
    if branches.len() == 1 {
        branches.into_iter().next()
    } else {
        None
    }
}

impl Policy {
    /// `need` over the given area rows (PL-0b #1014): the first area row with the same
    /// `(path, action)` answers its `need`; otherwise `need_table` answers. `need` is
    /// `need_with(AREA_ROWS, …)`.
    pub fn need_with(
        areas: &[&[crate::domain::policy::areas::AreaRow]],
        path: EntryPath,
        action: &Action,
    ) -> Option<Need> {
        let act = action.as_str();
        for area in areas {
            for row in *area {
                if row.path == path && row.action == act {
                    return Some(row.need);
                }
            }
        }
        Self::need_table(path, action)
    }

    /// The route table itself (§4.2), without the area rows. PL-0b moves today's `need` match
    /// here unchanged.
    pub fn need_table(path: EntryPath, action: &Action) -> Option<Need> {
        let act = action.as_str();
        match path {
            EntryPath::Rpc => match act {
                "adminSnapshot" | "adminDeskConfig" | "getConnectionStatus" => {
                    Some(Need::Permission(Permission::CompatRead))
                }
                "me" => Some(Need::Permission(Permission::SignedIn)),
                "reserveCustomer" | "saveTransaction" => {
                    Some(Need::Permission(Permission::ExchangeTransact))
                }
                "rateDraft" => Some(Need::Permission(Permission::RatesDraft)),
                "adminCreateBranch" => Some(Need::Permission(Permission::BranchesManage)),
                "adminAbandonRequest" => Some(Need::Permission(Permission::RequestAbandon)),
                "ratePolicySave" => Some(Need::Permission(Permission::RatePolicySave)),
                "approvalDecide" | "approvalRead" => {
                    Some(Need::Permission(Permission::ApprovalsDecide))
                }
                "reversalRequest" => Some(Need::Permission(Permission::ReversalRequest)),
                "consentPolicy" => Some(Need::Permission(Permission::ConsentPolicyManage)),
                _ => None,
            },
            EntryPath::StaffRpc => match act {
                "adminSnapshot" | "getConnectionStatus" => {
                    Some(Need::Permission(Permission::CompatRead))
                }
                "adminDeskConfig" | "reserveCustomer" | "saveTransaction" => {
                    Some(Need::Permission(Permission::ExchangeTransact))
                }
                "rateDraft" => Some(Need::Permission(Permission::RatesDraft)),
                "documents" => Some(Need::Permission(Permission::IdentityDocumentRead)),
                _ => None,
            },
            EntryPath::Upload => Some(Need::Permission(Permission::ProofUpload)),
            EntryPath::StaffUpload => Some(Need::Permission(Permission::ProofUpload)),
            EntryPath::File => Some(Need::Permission(Permission::FileRead)),
            EntryPath::StaffFile => Some(Need::Permission(Permission::IdentityDocumentRead)),
            EntryPath::StaffDesk => Some(Need::Permission(Permission::ExchangeTransact)),
            EntryPath::Devices => match act {
                "setPrinter" | "setDrawer" => Some(Need::Permission(Permission::SystemSettings)),
                "queue" => Some(Need::Permission(Permission::DeviceQueue)),
                "process" | "retry" | "drawerObservation" => {
                    Some(Need::Permission(Permission::DeviceProcess))
                }
                "printerOnline" => Some(Need::Permission(Permission::DevicePrinterOnline)),
                _ => Some(Need::Permission(Permission::SignedIn)),
            },
            EntryPath::Kiosk => match act {
                // R-8 (owner Q12, design kiosk-shared-phone.md §4.2 D19): only staff issue numbers, so the
                // kiosk `confirm` needs `QueueOperate`.
                "rates" | "quote" | "status" | "customerReceipt" | "customerCancel"
                | "settings" => Some(Need::Public),
                "manage" | "configure" | "configureSettings" => {
                    Some(Need::Permission(Permission::RatesDraft))
                }
                "notifications" | "list" => Some(Need::Permission(Permission::SignedIn)),
                "confirm" | "identityChecked" | "publishReceipt" | "get" | "accept" | "cancel"
                | "issueByToken" | "findByToken" => {
                    Some(Need::Permission(Permission::QueueOperate))
                }
                _ => None,
            },
            EntryPath::OwnerRoute => match act {
                "/api/reference-rates" => Some(Need::Permission(Permission::ReferenceRatesRead)),
                "queueTicketTake" => Some(Need::Permission(Permission::CustomerSelf)),
                _ => Some(Need::Permission(Permission::UsersManage)),
            },
            EntryPath::UserRoute => match act {
                "documents" => Some(Need::Permission(Permission::DocumentsList)),
                _ => Some(Need::Permission(Permission::SignedIn)),
            },
            EntryPath::Custody => Some(Need::Custody),
            EntryPath::PaymentMethods => match act {
                "save" => Some(Need::Permission(Permission::SystemSettings)),
                "list" => Some(Need::Permission(Permission::SignedIn)),
                _ => None,
            },
            EntryPath::DrawerPassport => Some(Need::Permission(Permission::DrawerPassport)),
            EntryPath::DeviceSetup => Some(Need::Permission(Permission::SystemSettings)),
        }
    }

    /// The row of the route table for `action` on `path` (§4.2); `None` = no row.
    pub fn need(path: EntryPath, action: &Action) -> Option<Need> {
        Self::need_with(crate::domain::policy::areas::AREA_ROWS, path, action)
    }

    /// Whether `actor` may ask for `action` on `path`, naming `named`, at `now`; and on which branch
    /// (§3.5 `Grant`). A refusal is said in the words of its path.
    pub fn authorize(
        actor: Option<&Who>,
        path: EntryPath,
        action: &Action,
        named: Option<&BranchId>,
        now: DateTime<Utc>,
    ) -> Result<Grant, PolicyError> {
        Self::authorize_with(
            crate::domain::policy::areas::AREA_ROWS,
            actor,
            path,
            action,
            named,
            now,
        )
    }

    /// `authorize` over the given area rows. `Policy::authorize` is `authorize_with(AREA_ROWS, …)`.
    pub fn authorize_with(
        areas: &[&[crate::domain::policy::areas::AreaRow]],
        actor: Option<&Who>,
        path: EntryPath,
        action: &Action,
        named: Option<&BranchId>,
        now: DateTime<Utc>,
    ) -> Result<Grant, PolicyError> {
        let need = Self::need_with(areas, path, action);

        // 1. Need::Public -> Grant
        if need == Some(Need::Public) {
            let branch = named
                .cloned()
                .or_else(|| actor.and_then(actor_single_branch))
                .unwrap_or_else(|| BranchId::new("MAIN"));
            return Ok(Grant {
                branch,
                roles_used: vec![],
            });
        }

        // 2. No actor -> path's sign-in sentence
        let Some(actor) = actor else {
            return match path {
                EntryPath::StaffRpc
                | EntryPath::StaffDesk
                | EntryPath::StaffFile
                | EntryPath::StaffUpload => Err(PolicyError::EmployeeAuthRequired),
                EntryPath::Kiosk => match action.as_str() {
                    "manage" | "configure" | "configureSettings" => {
                        Err(PolicyError::KioskOwnerOrRates)
                    }
                    _ => Err(PolicyError::KioskStaff),
                },
                _ => Err(PolicyError::SignIn),
            };
        };

        // 3. No row (need is None) -> path's unknown-name sentence
        let Some(need) = need else {
            return match path {
                EntryPath::Rpc => {
                    let is_executive = actor.roles.iter().any(|r| {
                        r.valid_from <= now
                            && r.valid_to.is_none_or(|to| now < to)
                            && r.role == BpRole::Executive
                    });
                    if is_executive {
                        Err(PolicyError::InvalidRequest)
                    } else {
                        Err(PolicyError::RoleNotAllowedHere)
                    }
                }
                EntryPath::StaffRpc => Err(PolicyError::RoleNotAllowed),
                EntryPath::Kiosk => {
                    let has_queue_operate = actor.roles.iter().any(|r| {
                        r.valid_from <= now
                            && r.valid_to.is_none_or(|to| now < to)
                            && RoleGrants::holds(r.group_code(), Permission::QueueOperate)
                    });
                    if has_queue_operate {
                        Err(PolicyError::KioskUnknown)
                    } else {
                        Err(PolicyError::KioskStaff)
                    }
                }
                _ => Err(PolicyError::InvalidRequest),
            };
        };

        // 5. Need::Custody -> refused with InvalidRequest
        let single;
        let permissions: &[Permission] = match need {
            Need::Public => unreachable!(),
            Need::Custody => return Err(PolicyError::InvalidRequest),
            Need::Permission(p) => {
                single = [p];
                &single
            }
            Need::AnyOf(ps) => ps,
        };

        // 6. Authorizer::check
        let target = Target::new(
            &actor.organization_id,
            named.cloned(),
            actor.customer_id.as_deref(),
        );
        let non_customer;
        let check_actor = if path == EntryPath::Rpc && action.as_str() == "me" {
            non_customer = Who {
                roles: actor
                    .roles
                    .iter()
                    .filter(|r| r.role != BpRole::Customer)
                    .cloned()
                    .collect(),
                ..actor.clone()
            };
            &non_customer
        } else {
            actor
        };

        let mut saw_out_of_scope = false;
        for &perm in permissions {
            match Authorizer::check(Some(check_actor), perm, &target, now) {
                Ok(grant) => return Ok(grant),
                Err(Refusal::SignIn) => return Err(PolicyError::SignIn),
                Err(Refusal::OutOfScope) => {
                    saw_out_of_scope = true;
                }
                Err(Refusal::RoleNotAllowed) => {}
            }
        }

        let refusal = if saw_out_of_scope {
            Refusal::OutOfScope
        } else {
            Refusal::RoleNotAllowed
        };

        match refusal {
            Refusal::SignIn => Err(PolicyError::SignIn),
            Refusal::OutOfScope => Err(PolicyError::OutOfScope),
            Refusal::RoleNotAllowed => match path {
                EntryPath::Rpc => Err(PolicyError::RoleNotAllowedHere),
                EntryPath::StaffRpc => match action.as_str() {
                    "adminDeskConfig" => Err(PolicyError::FrontOnly),
                    "documents" => Err(PolicyError::DocumentsNotReadable),
                    _ => Err(PolicyError::RoleNotAllowed),
                },
                EntryPath::StaffDesk => Err(PolicyError::FrontOnly),
                EntryPath::Upload | EntryPath::StaffUpload => Err(PolicyError::UploadNotAllowed),
                EntryPath::File => Err(PolicyError::NotFound),
                EntryPath::StaffFile => Err(PolicyError::FilesNotReadable),
                EntryPath::OwnerRoute | EntryPath::DeviceSetup => Err(PolicyError::OwnerOnly),
                EntryPath::Devices => Err(PolicyError::DeviceStepNotAllowed),
                EntryPath::Kiosk => match action.as_str() {
                    "manage" | "configure" | "configureSettings" => {
                        Err(PolicyError::KioskOwnerOrRates)
                    }
                    _ => Err(PolicyError::KioskStaff),
                },
                EntryPath::PaymentMethods => match action.as_str() {
                    "save" => Err(PolicyError::PaymentOwnerOnly),
                    _ => Err(PolicyError::RoleNotAllowed),
                },
                EntryPath::DrawerPassport => Err(PolicyError::DrawerRoleNotAllowed),
                _ => Err(PolicyError::RoleNotAllowed),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::access::ScopedRole;
    use chrono::Duration;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn who(roles: &[(BpRole, Option<&str>)]) -> Who {
        let since = at() - Duration::days(1);
        Who::new(
            "u1",
            "org",
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), since, None))
                .collect(),
            None,
        )
    }

    #[test]
    fn me_row_needs_signed_in_and_allows_marketing_staff_and_owner_while_refusing_customer() {
        let me = Action::new("me");
        assert_eq!(
            Policy::need(EntryPath::Rpc, &me),
            Some(Need::Permission(Permission::SignedIn))
        );
        assert!(!RoleGrants::holds("MARKETING", Permission::CompatRead));

        let mkt = who(&[(BpRole::Marketing, Some("MAIN"))]);
        assert!(Policy::authorize(Some(&mkt), EntryPath::Rpc, &me, None, at()).is_ok());
        assert_eq!(
            Policy::authorize(
                Some(&mkt),
                EntryPath::Rpc,
                &Action::new("adminSnapshot"),
                None,
                at()
            ),
            Err(PolicyError::RoleNotAllowedHere)
        );

        let teller = who(&[(BpRole::Teller, Some("MAIN"))]);
        assert!(Policy::authorize(Some(&teller), EntryPath::Rpc, &me, None, at()).is_ok());

        let owner = who(&[(BpRole::Executive, None), (BpRole::Admin, None)]);
        assert!(Policy::authorize(Some(&owner), EntryPath::Rpc, &me, None, at()).is_ok());

        let customer = who(&[(BpRole::Customer, None)]);
        assert_eq!(
            Policy::authorize(Some(&customer), EntryPath::Rpc, &me, None, at()),
            Err(PolicyError::RoleNotAllowedHere)
        );

        assert_eq!(
            Policy::authorize(None, EntryPath::Rpc, &me, None, at()),
            Err(PolicyError::SignIn)
        );
    }
}
