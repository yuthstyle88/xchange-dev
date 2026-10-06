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
    StaffDesk,
    Devices,
    Kiosk,
    OwnerRoute,
    UserRoute,
    Custody,
}
