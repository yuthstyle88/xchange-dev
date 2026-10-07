//! `xc-kiosk` — kiosk bounded context.

pub mod shared {
    pub mod error {
        pub use pos_error::{AppError, AppResult};
    }
}

pub mod application;
pub mod domain;
pub mod infrastructure;
