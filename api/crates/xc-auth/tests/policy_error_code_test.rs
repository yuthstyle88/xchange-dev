use xc_auth::domain::policy::value_objects::policy_error::PolicyError;

#[test]
fn policy_error_code_matches_v3_table() {
    let cases = [
        (PolicyError::SignIn, "signIn"),
        (PolicyError::OwnerOnly, "ownerOnly"),
        (PolicyError::EmployeeAuthRequired, "employeeAuthRequired"),
        (PolicyError::InvalidRequest, "invalidRequest"),
        (PolicyError::RoleNotAllowed, "roleNotAllowed"),
        (PolicyError::RoleNotAllowedHere, "roleNotAllowedHere"),
        (PolicyError::OwnerEnables, "ownerEnables"),
        (PolicyError::FrontOnly, "frontOnly"),
        (PolicyError::DocumentsNotReadable, "documentsNotReadable"),
        (PolicyError::FilesNotReadable, "filesNotReadable"),
        (PolicyError::UploadNotAllowed, "uploadNotAllowed"),
        (PolicyError::NotFound, "notFound"),
        (PolicyError::DeviceStepNotAllowed, "deviceStepNotAllowed"),
        (PolicyError::KioskOwnerOrRates, "kioskOwnerOrRates"),
        (PolicyError::KioskStaff, "kioskStaff"),
        (PolicyError::KioskUnknown, "kioskUnknown"),
        (PolicyError::SignNotAllowed, "signNotAllowed"),
        (PolicyError::BackendUsesCommit, "backendUsesCommit"),
        (PolicyError::EmployeeAccount, "employeeAccount"),
        (PolicyError::ClockInFirst, "clockInFirst"),
        (PolicyError::PaymentOwnerOnly, "paymentOwnerOnly"),
        (PolicyError::DrawerRoleNotAllowed, "drawerRoleNotAllowed"),
        (PolicyError::OutOfScope, "outOfScope"),
    ];

    for (err, expected) in cases {
        match &err {
            PolicyError::SignIn
            | PolicyError::OwnerOnly
            | PolicyError::EmployeeAuthRequired
            | PolicyError::InvalidRequest
            | PolicyError::RoleNotAllowed
            | PolicyError::RoleNotAllowedHere
            | PolicyError::OwnerEnables
            | PolicyError::FrontOnly
            | PolicyError::DocumentsNotReadable
            | PolicyError::FilesNotReadable
            | PolicyError::UploadNotAllowed
            | PolicyError::NotFound
            | PolicyError::DeviceStepNotAllowed
            | PolicyError::KioskOwnerOrRates
            | PolicyError::KioskStaff
            | PolicyError::KioskUnknown
            | PolicyError::SignNotAllowed
            | PolicyError::BackendUsesCommit
            | PolicyError::EmployeeAccount
            | PolicyError::ClockInFirst
            | PolicyError::PaymentOwnerOnly
            | PolicyError::DrawerRoleNotAllowed
            | PolicyError::OutOfScope => (),
        }
        assert_eq!(err.code(), expected, "mismatch for {err:?}");
    }
}
