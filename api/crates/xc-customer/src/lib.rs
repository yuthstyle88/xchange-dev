//! `xc-customer` — customer identity, consent, and retention bounded context.

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod pii_keys;

pub use application::customer::phone_verification_service::{
    PhoneConfirmResult, PhoneStartResult, PhoneVerificationError, PhoneVerificationService,
};
pub use application::customer::service::{
    BAD_DATE, BAD_MODE, BAD_TEXT, COUNTER_EXHAUSTED, CustomerService, INVALID_NATIONALITY,
    NO_SUCH_BRANCH, NOT_THIS_PASSPORT,
};
pub use domain::customer::commands::reserve_customer::ReserveCustomer;
pub use domain::customer::entities::customer::Customer;
pub use domain::customer::entities::customer_identity::CustomerIdentity;
pub use domain::customer::entities::reservation::Reservation;
pub use domain::customer::repositories::customer_repository::CustomerRepository;
pub use domain::customer::value_objects::customer_details::CustomerDetails;
pub use domain::phone_otp_policy::PhoneOtpPolicy;
pub use domain::phone_verification::{PhoneVerification, PhoneVerificationStatus};
pub use domain::sms_sender::SmsSender;
pub use infrastructure::customer::customer_repository::PgCustomerRepository;
pub use infrastructure::customer::phone_verification_repository::PgPhoneVerificationRepository;
pub use pii_keys::{PiiKeysConflict, get_pii_keys, install_pii_keys};
