//! The scanner seam (design §6): one trait, one file per platform. A later Windows or Linux
//! counter is one more file.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub mod fake;
pub mod mac;

/// Why a scan gave no image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanFailure {
    NoScanner,
    Busy,
    Timeout,
    CoverOpen,
    /// The text goes to the log, never to the page.
    Device(String),
}

impl ScanFailure {
    /// The string `xcCounter.scanPassport()` answers as `reason` (§3).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NoScanner => "noScanner",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::CoverOpen => "coverOpen",
            Self::Device(_) => "device",
        }
    }
}

/// What one scan asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRequest {
    pub dpi: u32,
    pub color: bool,
    pub timeout: Duration,
    pub whole_bed: bool,
}

impl ScanRequest {
    /// §3: one page, colour, 300 dpi, whole bed, 60 s.
    pub fn passport() -> Self {
        Self {
            dpi: 300,
            color: true,
            timeout: Duration::from_secs(60),
            whole_bed: true,
        }
    }
}

/// `Ok(())` when a scanner is there to use.
pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ScanFailure>> + Send + 'a>>;
/// `Ok` is the JPEG.
pub type ScanFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, ScanFailure>> + Send + 'a>>;

/// A scanner. Boxed futures keep it object-safe, so the app can hold an `Arc<dyn Scanner>`.
pub trait Scanner: Send + Sync {
    fn probe(&self) -> ProbeFuture<'_>;
    /// One scan at a time: a second call while one is in flight answers [`ScanFailure::Busy`].
    fn scan(&self, request: ScanRequest) -> ScanFuture<'_>;
}
