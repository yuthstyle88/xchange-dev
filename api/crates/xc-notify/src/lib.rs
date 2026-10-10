//! `xc-notify` — customer notifications (design `docs/design/blueprint/customer-app.md` P8–P10,
//! §4.1–§4.3, §5). Producers reach it only through `xc_kernel::notifications::Notifications`.

pub mod application;
pub mod domain;
pub mod templates;
