//! Entry path value object.

/// An entry path into the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryPath {
    Rpc,
    StaffRpc,
    Upload,
    StaffUpload,
    File,
    StaffFile,
    Devices,
    Kiosk,
    OwnerRoute,
    UserRoute,
    PaymentMethods,
    DrawerPassport,
    DeviceSetup,
}
