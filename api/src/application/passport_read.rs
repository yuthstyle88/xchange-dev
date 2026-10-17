//! The desk's passport read (design `docs/design/recognize-passport-read.md` §3): the one trait the
//! route calls, and the values that cross it.

use async_trait::async_trait;

/// The three image types Recognize accepts (`ai-reader.md` D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageMime {
    Jpeg,
    Png,
    Webp,
}

/// Why a `Content-Type` is not an accepted image type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeRefusal {
    /// `application/pdf`: the desk renders the page to an image first (design R-6).
    Pdf,
    /// Anything else, or nothing.
    Other,
}

impl ImageMime {
    /// Reads a `Content-Type` value on its type and subtype only (lower-cased, before any `;`).
    pub fn parse(content_type: &str) -> Result<Self, MimeRefusal> {
        let base = content_type.split(';').next().unwrap_or("").trim();
        match base.to_ascii_lowercase().as_str() {
            "image/jpeg" => Ok(Self::Jpeg),
            "image/png" => Ok(Self::Png),
            "image/webp" => Ok(Self::Webp),
            "application/pdf" => Err(MimeRefusal::Pdf),
            _ => Err(MimeRefusal::Other),
        }
    }

    /// `image/jpeg`, `image/png` or `image/webp`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// The image of one read, held in memory for one request only (design R-5).
#[derive(Clone)]
pub struct PassportImage {
    mime: ImageMime,
    bytes: Vec<u8>,
}

impl PassportImage {
    pub fn new(mime: ImageMime, bytes: Vec<u8>) -> Self {
        Self { mime, bytes }
    }

    pub fn mime(&self) -> ImageMime {
        self.mime
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Prints the mime type and the byte count, never the bytes (design §5).
impl std::fmt::Debug for PassportImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassportImage")
            .field("mime", &self.mime)
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

/// The AI Recognize is asked to use (design R-1: Gemini or the local AI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
    Gemini,
    Ollama,
}

impl AiProvider {
    /// The wire value: `gemini` or `ollama`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
        }
    }
}

/// Where Recognize says the read ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiLocation {
    Cloud,
    Local,
}

/// `ai-reader.md` §6 `visual_zone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualZone {
    Same,
    Differs,
    Unchecked,
}

/// Recognize's passport `result` (`ai-reader.md` §6). `None` = not read.
#[derive(Clone, PartialEq, Eq)]
pub struct PassportResult {
    pub name: Option<String>,
    pub passport_no: Option<String>,
    pub nationality: Option<String>,
    pub birth_date: Option<String>,
    pub expiry_date: Option<String>,
    pub score: u8,
    pub visual_zone: VisualZone,
}

/// Never prints a field value (design §5).
impl std::fmt::Debug for PassportResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassportResult")
            .field("score", &self.score)
            .field("visual_zone", &self.visual_zone)
            .finish_non_exhaustive()
    }
}

/// One answer of Recognize (`ai-reader.md` §4, 200).
#[derive(Clone, PartialEq, Eq)]
pub struct PassportRead {
    pub provider: AiProvider,
    pub model: String,
    pub location: AiLocation,
    /// `None` = Recognize read the image but its checks refused it; not an error.
    pub result: Option<PassportResult>,
    pub auto_acceptable: bool,
}

/// Prints the provider, model, location, score and `auto_acceptable`, with no field value (design §5).
impl std::fmt::Debug for PassportRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassportRead")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("location", &self.location)
            .field("score", &self.result.as_ref().map(|r| r.score))
            .field("auto_acceptable", &self.auto_acceptable)
            .finish()
    }
}

/// Why one read gave no answer (design §4.4, second table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    NotConfigured,
    Busy { retry_after_secs: u32 },
    Failed,
    Timeout,
    ImageInvalid,
    ImageTooLarge,
    PdfNotAccepted,
    Internal,
}

/// Reads one passport image, once (design R-2: no retry, no fallback).
#[async_trait]
pub trait PassportReader: Send + Sync {
    /// `false` only for [`UnconfiguredReader`]: the route answers 503 before reading the body.
    fn configured(&self) -> bool {
        true
    }

    async fn read(&self, image: &PassportImage) -> Result<PassportRead, ReadError>;
}

/// The reader installed when `Settings.recognize` is `Err`: every read is `NotConfigured`.
pub struct UnconfiguredReader;

#[async_trait]
impl PassportReader for UnconfiguredReader {
    fn configured(&self) -> bool {
        false
    }

    async fn read(&self, _image: &PassportImage) -> Result<PassportRead, ReadError> {
        Err(ReadError::NotConfigured)
    }
}
