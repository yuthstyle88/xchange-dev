//! Notify infrastructure client (CU-P1, design §6.1, §6.3).

pub mod notify_email_sender;
pub mod notify_sms_sender;

pub use notify_email_sender::NotifyEmailSender;
pub use notify_sms_sender::NotifySmsSender;
