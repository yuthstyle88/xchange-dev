//! How the kick reaches the printer.

use std::future::Future;

/// What a transport learned while sending.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendReceipt {
    /// CUPS's job id (`lp`'s `request id is <id> (…)`); `None` for TCP.
    pub cups_job: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// `lpstat -o <queue>` lists a pending job: nothing was sent.
    QueueBusy,
    /// The send failed; the text says how (never a secret).
    Failed(String),
}

impl TransportError {
    /// The `reason` a report carries: `QUEUE_BUSY` or `SEND_FAILED`.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::QueueBusy => "QUEUE_BUSY",
            Self::Failed(_) => "SEND_FAILED",
        }
    }
}

/// Sends the bytes it is given to one printer. It never builds bytes itself.
pub trait DrawerTransport {
    fn send(
        &self,
        bytes: &[u8],
    ) -> impl Future<Output = Result<SendReceipt, TransportError>> + Send;
}
