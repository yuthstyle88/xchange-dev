pub mod customer_details;
pub mod display_name;
pub use customer_details::CustomerDetails;
pub use display_name::{DISPLAY_NAME_MAX_CHARS, exceeds_display_name_limit};
