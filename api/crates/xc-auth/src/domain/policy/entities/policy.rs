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
                | "adminOpenShift"
                | "adminCloseShift"
                | "adminMoveStock"
                | "adminSaveRate"
                | "ratePolicySave"
                | "rateDraft"
                | "approvalDecide"
                | "approvalRead"
                | "adminCreateBranch"
                | "adminAbandonRequest"
                | "getConnectionStatus"
                | "reserveCustomer"
                | "saveTransaction" => Ok(()),
                _ => Err(PolicyError::InvalidRequest),
            },
            ActorKind::Staff => {
                let role = actor.role();
                let allowed = match act {
                    "adminSnapshot" | "adminDeskConfig" | "getConnectionStatus" => true,
                    "reserveCustomer" | "saveTransaction" => role == Role::Front,
                    "adminSaveRate" | "rateDraft" | "ratePolicySave" => role == Role::Rates,
                    "adminOpenShift" | "adminCloseShift" => {
                        role == Role::Backend || role == Role::Manager
                    }
                    "adminMoveStock" => role == Role::Finance,
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

        // 1. the name is not in known common actions, not in role's write list,
        // and does not start with ops -> R5
        let in_write_list = match role {
            Role::Front => act == "reserveCustomer" || act == "saveTransaction",
            Role::Rates => act == "adminSaveRate" || act == "rateDraft",
            _ => false,
        };
        let recognized = act == "adminSnapshot"
            || act == "adminDeskConfig"
            || act == "getConnectionStatus"
            || act == "documents"
            || in_write_list
            || act.starts_with("ops");

        if !recognized {
            return Err(PolicyError::RoleNotAllowed);
        }

        // 2. the name is opsEnable or opsSetEntity -> R7
        if act == "opsEnable" || act == "opsSetEntity" {
            return Err(PolicyError::OwnerEnables);
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

    fn allow_custody(actor: Option<&Actor>, action: &Action) -> Result<(), PolicyError> {
        let Some(actor) = actor else {
            return Err(PolicyError::SignIn);
        };
        let act = action.as_str();

        match actor.kind() {
            ActorKind::Owner => match act {
                "opsSetEntity" | "opsEnable" => Ok(()),
                "opsClock" => Err(PolicyError::EmployeeAccount),
                _ => Err(PolicyError::ClockInFirst),
            },
            ActorKind::Staff => {
                let role = actor.role();
                match act {
                    "opsSetEntity" | "opsEnable" => Err(PolicyError::RoleNotAllowed),
                    "opsClock" => Ok(()),
                    "opsBeginTransaction" => {
                        if role == Role::Front {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsTarget" | "opsReconcile" => {
                        if role == Role::Finance {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsStartService" | "opsCutoff" | "opsDispatch" | "opsCommit" => {
                        if role == Role::Backend {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsPropose" => {
                        if role == Role::Front || role == Role::Finance {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsHold" => {
                        if role == Role::Front || role == Role::Monitor {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsRelease" => {
                        if role == Role::Monitor {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsReject" => {
                        if role == Role::Monitor || role == Role::Backend {
                            Ok(())
                        } else {
                            Err(PolicyError::RoleNotAllowed)
                        }
                    }
                    "opsSign" => match role {
                        Role::Front | Role::Finance | Role::Monitor | Role::Manager => Ok(()),
                        Role::Backend => Err(PolicyError::BackendUsesCommit),
                        Role::Rates | Role::System | Role::Documents | Role::DocumentReview => {
                            Err(PolicyError::SignNotAllowed)
                        }
                        Role::Owner => Err(PolicyError::ClockInFirst),
                    },
                    _ => Err(PolicyError::InvalidRequest),
                }
            }
        }
    }
}
