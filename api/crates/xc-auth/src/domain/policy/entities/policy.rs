//! Policy entity.

use xc_kernel::ids::BranchId;

use crate::domain::policy::value_objects::action::Action;
use crate::domain::policy::value_objects::actor::Actor;
use crate::domain::policy::value_objects::actor_kind::ActorKind;
use crate::domain::policy::value_objects::entry_path::EntryPath;
use crate::domain::policy::value_objects::policy_error::PolicyError;
use crate::domain::policy::value_objects::role::Role;

/// Authorisation policy engine.
pub struct Policy;

impl Policy {
    /// Determines whether the caller may perform `action` on `path`.
    pub fn allow(
        actor: Option<&Actor>,
        path: EntryPath,
        action: &Action,
    ) -> Result<(), PolicyError> {
        match path {
            EntryPath::Rpc => Self::allow_rpc(actor, action),
            EntryPath::StaffRpc => Self::allow_staff_rpc(actor, action),
            EntryPath::Upload => Self::allow_upload(actor),
            EntryPath::StaffUpload => Self::allow_staff_upload(actor),
            EntryPath::File => Self::allow_file(actor),
            EntryPath::StaffFile => Self::allow_staff_file(actor),
            EntryPath::StaffDesk => Self::allow_staff_desk(actor),
            EntryPath::OwnerRoute => Self::allow_owner_route(actor),
            EntryPath::UserRoute => Self::allow_user_route(actor),
            EntryPath::Devices => Self::allow_devices(actor, action),
            EntryPath::Kiosk => Self::allow_kiosk(actor, action),
            EntryPath::Custody => Self::allow_custody(actor, action),
            EntryPath::PaymentMethods => Self::allow_payment_methods(actor, action),
            EntryPath::DrawerPassport => Self::allow_drawer_passport(actor),
            EntryPath::DeviceSetup => Self::allow_owner_route(actor),
            EntryPath::StaffBookings => Self::allow_staff_bookings(actor, action),
        }
    }

    /// Determines the branch the operation runs on.
    pub fn branch(actor: Option<&Actor>, named: Option<&BranchId>) -> BranchId {
        if let Some(actor) = actor {
            if let Some(branch_id) = actor.branch_id() {
                return branch_id.clone();
            }
        }
        match named {
            Some(b) => b.clone(),
            None => BranchId::new("MAIN"),
        }
    }

    // ── Rpc ──────────────────────────────────────────────────────────────────

    fn allow_rpc(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        let act = action.as_str();
        match actor.kind() {
            ActorKind::Owner => match act {
                "adminSnapshot"
                | "adminDeskConfig"
                | "ratePolicySave"
                | "rateDraft"
                | "approvalDecide"
                | "approvalRead"
                | "adminCreateBranch"
                | "adminAbandonRequest"
                | "getConnectionStatus"
                | "reserveCustomer"
                | "saveTransaction"
                | "consentPolicy" => Ok(()),
                _ => Err(PolicyError::InvalidRequest),
            },
            ActorKind::Staff => {
                let role = actor.role();
                let allowed = match act {
                    "adminSnapshot" | "adminDeskConfig" | "getConnectionStatus" => true,
                    "reserveCustomer" | "saveTransaction" => role == Role::Front,
                    "rateDraft" | "ratePolicySave" => role == Role::Rates,
                    "approvalDecide" | "approvalRead" => {
                        role == Role::Backend || role == Role::Manager || role == Role::Finance
                    }
                    "reversalRequest" => {
                        role == Role::Front || role == Role::Manager || role == Role::Backend
                    }
                    _ => false,
                };
                if allowed {
                    Ok(())
                } else {
                    Err(PolicyError::RoleNotAllowedHere)
                }
            }
        }
    }

    // ── StaffRpc ─────────────────────────────────────────────────────────────

    fn allow_staff_rpc(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::EmployeeAuthRequired);
        };
        if actor.kind() == ActorKind::Owner {
            return Err(PolicyError::EmployeeAuthRequired);
        }

        let act = action.as_str();
        let role = actor.role();

        // 1. the name is not in known common actions and not in role's write list -> R5
        let in_write_list = match role {
            Role::Front => act == "reserveCustomer" || act == "saveTransaction",
            Role::Rates => act == "rateDraft",
            _ => false,
        };
        let recognized = act == "adminSnapshot"
            || act == "adminDeskConfig"
            || act == "getConnectionStatus"
            || act == "documents"
            || in_write_list;

        if !recognized {
            return Err(PolicyError::RoleNotAllowed);
        }

        // 3. the name is adminDeskConfig and role is not FR -> R8
        if act == "adminDeskConfig" && role != Role::Front {
            return Err(PolicyError::FrontOnly);
        }

        // 4. the name is documents and role is RT or SY -> R9
        if act == "documents" && (role == Role::Rates || role == Role::System) {
            return Err(PolicyError::DocumentsNotReadable);
        }

        // 5. otherwise allowed
        Ok(())
    }

    // ── One-purpose paths ────────────────────────────────────────────────────

    fn allow_upload(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        match actor.kind() {
            ActorKind::Owner => Ok(()),
            ActorKind::Staff => match actor.role() {
                Role::Front | Role::Finance => Ok(()),
                _ => Err(PolicyError::UploadNotAllowed),
            },
        }
    }

    fn allow_staff_upload(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::EmployeeAuthRequired);
        };
        match actor.kind() {
            ActorKind::Owner => Err(PolicyError::EmployeeAuthRequired),
            ActorKind::Staff => match actor.role() {
                Role::Front | Role::Finance => Ok(()),
                _ => Err(PolicyError::UploadNotAllowed),
            },
        }
    }

    fn allow_file(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        match actor.kind() {
            ActorKind::Owner => Ok(()),
            ActorKind::Staff => match actor.role() {
                Role::Rates | Role::System => Err(PolicyError::NotFound),
                _ => Ok(()),
            },
        }
    }

    fn allow_staff_file(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::EmployeeAuthRequired);
        };
        match actor.kind() {
            ActorKind::Owner => Err(PolicyError::EmployeeAuthRequired),
            ActorKind::Staff => match actor.role() {
                Role::Rates | Role::System => Err(PolicyError::FilesNotReadable),
                _ => Ok(()),
            },
        }
    }

    fn allow_staff_desk(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::EmployeeAuthRequired);
        };
        match actor.kind() {
            ActorKind::Owner => Err(PolicyError::EmployeeAuthRequired),
            ActorKind::Staff => match actor.role() {
                Role::Front => Ok(()),
                _ => Err(PolicyError::FrontOnly),
            },
        }
    }

    fn allow_owner_route(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        match actor.kind() {
            ActorKind::Owner => Ok(()),
            ActorKind::Staff => Err(PolicyError::OwnerOnly),
        }
    }

    fn allow_user_route(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(_) = actor else {
            return Err(PolicyError::SignIn);
        };
        Ok(())
    }

    // ── Devices ──────────────────────────────────────────────────────────────

    fn allow_devices(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        let act = action.as_str();
        let role = actor.role();
        let is_owner = actor.kind() == ActorKind::Owner;

        match act {
            "setPrinter" | "setDrawer" => {
                if is_owner {
                    Ok(())
                } else {
                    Err(PolicyError::DeviceStepNotAllowed)
                }
            }
            "queue" => {
                if is_owner || role == Role::Front || role == Role::Backend || role == Role::Manager
                {
                    Ok(())
                } else {
                    Err(PolicyError::DeviceStepNotAllowed)
                }
            }
            "printerOnline" => {
                if is_owner || role == Role::Backend {
                    Ok(())
                } else {
                    Err(PolicyError::DeviceStepNotAllowed)
                }
            }
            "process" | "retry" | "drawerObservation" => {
                if is_owner || role == Role::Front || role == Role::Backend {
                    Ok(())
                } else {
                    Err(PolicyError::DeviceStepNotAllowed)
                }
            }
            // Unknown or other name: allowed for anybody signed in
            _ => Ok(()),
        }
    }

    // ── Kiosk ────────────────────────────────────────────────────────────────

    fn allow_kiosk(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let act = action.as_str();

        // 1. manage, configure: OWN and RT allowed; everybody else -> R14
        if act == "manage" || act == "configure" {
            let Some(actor) = actor else {
                return Err(PolicyError::KioskOwnerOrRates);
            };
            if actor.kind() == ActorKind::Owner || actor.role() == Role::Rates {
                return Ok(());
            }
            return Err(PolicyError::KioskOwnerOrRates);
        }

        // 2. rates, quote, confirm, status, customerReceipt: allowed for everybody
        if act == "rates"
            || act == "quote"
            || act == "confirm"
            || act == "status"
            || act == "customerReceipt"
        {
            return Ok(());
        }

        // 3. notifications, list: any signed-in caller; nobody -> R15
        if act == "notifications" || act == "list" {
            if actor.is_some() {
                return Ok(());
            }
            return Err(PolicyError::KioskStaff);
        }

        // 4. any other name: caller must be OWN, FR, BE, or MG, else -> R15
        let Some(actor) = actor else {
            return Err(PolicyError::KioskStaff);
        };
        let role = actor.role();
        let privileged = actor.kind() == ActorKind::Owner
            || role == Role::Front
            || role == Role::Backend
            || role == Role::Manager;

        if !privileged {
            return Err(PolicyError::KioskStaff);
        }

        // 5. then identityChecked, publishReceipt, get, accept -> allowed; else -> R16
        match act {
            "identityChecked" | "publishReceipt" | "get" | "accept" => Ok(()),
            _ => Err(PolicyError::KioskUnknown),
        }
    }

    // ── PaymentMethods ────────────────────────────────────────────────────────

    fn allow_payment_methods(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        let act = action.as_str();
        match act {
            "list" => Ok(()),
            "save" => {
                if actor.kind() == ActorKind::Owner {
                    Ok(())
                } else {
                    Err(PolicyError::PaymentOwnerOnly)
                }
            }
            _ => Err(PolicyError::InvalidRequest),
        }
    }

    // ── DrawerPassport ────────────────────────────────────────────────────────

    fn allow_drawer_passport(actor: Option<&Actor>) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        let privileged = actor.kind() == ActorKind::Owner
            || actor.role() == Role::Front
            || actor.role() == Role::Backend
            || actor.role() == Role::Manager;
        if privileged {
            Ok(())
        } else {
            Err(PolicyError::DrawerRoleNotAllowed)
        }
    }

    // ── StaffBookings ────────────────────────────────────────────────────────

    fn allow_staff_bookings(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::EmployeeAuthRequired);
        };
        if actor.kind() == ActorKind::Owner {
            return Err(PolicyError::EmployeeAuthRequired);
        }

        let role = actor.role();
        match role {
            Role::Front | Role::Backend | Role::Manager => {
                let act = action.as_str();
                match act {
                    "list" | "get" | "create" | "confirm" | "cancel" => Ok(()),
                    "member" | "settings" | "registrations" | "approveRegistration" => {
                        Err(PolicyError::OwnerOnly)
                    }
                    _ => Err(PolicyError::InvalidRequest),
                }
            }
            _ => Err(PolicyError::RoleNotAllowed),
        }
    }

    // ── Custody ──────────────────────────────────────────────────────────────

    fn allow_custody(actor: Option<&Actor>, _action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };

        match actor.kind() {
            ActorKind::Owner => Err(PolicyError::ClockInFirst),
            ActorKind::Staff => Err(PolicyError::InvalidRequest),
        }
    }
}

// ── OR-4 (#550): the route table onto `Authorizer` ─────────────────────────────
//
// Signatures only, from the ticket author of OR-4: the bodies are OR-4's to write
// (`api/crates/xc-auth/tests/policy_*_test.rs`, design `docs/design/blueprint/org-roles-branches.md`
// §3.5, §4.2).

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::domain::access::{
    Actor as Who, Authorizer, Grant, Permission, Refusal, Role as BpRole, RolePermissions, Target,
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
                "rates" | "quote" | "confirm" | "status" | "customerReceipt" => Some(Need::Public),
                "manage" | "configure" => Some(Need::Permission(Permission::RatesDraft)),
                "notifications" | "list" => Some(Need::Permission(Permission::SignedIn)),
                "identityChecked" | "publishReceipt" | "get" | "accept" => {
                    Some(Need::Permission(Permission::QueueOperate))
                }
                _ => None,
            },
            EntryPath::OwnerRoute => match act {
                "/api/v2/bookings" => Some(Need::Permission(Permission::BookingsAdmin)),
                "/api/reference-rates" => Some(Need::Permission(Permission::ReferenceRatesRead)),
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
            EntryPath::StaffBookings => match act {
                "list" | "get" | "create" | "confirm" | "cancel" => {
                    Some(Need::Permission(Permission::BookingsDesk))
                }
                "member" | "settings" | "registrations" | "approveRegistration" => {
                    Some(Need::Permission(Permission::BookingsAdmin))
                }
                _ => None,
            },
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
                | EntryPath::StaffUpload
                | EntryPath::StaffBookings => Err(PolicyError::EmployeeAuthRequired),
                EntryPath::Kiosk => match action.as_str() {
                    "manage" | "configure" => Err(PolicyError::KioskOwnerOrRates),
                    _ => Err(PolicyError::KioskStaff),
                },
                _ => Err(PolicyError::SignIn),
            };
        };

        // 3. StaffBookings without BookingsDesk -> RoleNotAllowed before the name is read (#410)
        if path == EntryPath::StaffBookings {
            let has_bookings_desk = actor.roles.iter().any(|r| {
                r.valid_from <= now
                    && r.valid_to.is_none_or(|to| now < to)
                    && RolePermissions::of(r.role).contains(&Permission::BookingsDesk)
            });
            if !has_bookings_desk {
                return Err(PolicyError::RoleNotAllowed);
            }
        }

        // 4. No row (need is None) -> path's unknown-name sentence
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
                            && RolePermissions::of(r.role).contains(&Permission::QueueOperate)
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

        let mut saw_out_of_scope = false;
        for &perm in permissions {
            match Authorizer::check(Some(actor), perm, &target, now) {
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
                    "manage" | "configure" => Err(PolicyError::KioskOwnerOrRates),
                    _ => Err(PolicyError::KioskStaff),
                },
                EntryPath::PaymentMethods => match action.as_str() {
                    "save" => Err(PolicyError::PaymentOwnerOnly),
                    _ => Err(PolicyError::RoleNotAllowed),
                },
                EntryPath::DrawerPassport => Err(PolicyError::DrawerRoleNotAllowed),
                EntryPath::StaffBookings => match action.as_str() {
                    "member" | "settings" | "registrations" | "approveRegistration" => {
                        Err(PolicyError::OwnerOnly)
                    }
                    _ => Err(PolicyError::RoleNotAllowed),
                },
                _ => Err(PolicyError::RoleNotAllowed),
            },
        }
    }
}
