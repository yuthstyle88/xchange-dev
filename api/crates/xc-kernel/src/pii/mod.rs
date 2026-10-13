//! PII core: identity types, normalisation, keys, AES-256-GCM cipher, keyed lookup hasher, and masking.

pub mod cipher;
pub mod derive;
pub mod error;
pub mod identity_type;
pub mod key_file;
pub mod lookup_hash;
pub mod masked;
pub mod normalize;
pub mod text_redactor;

pub use cipher::{PiiCipher, Sealed};
pub use derive::derive_subkey;
pub use error::PiiError;
pub use identity_type::{IdentityType, PiiKind};
pub use key_file::PiiKeys;
pub use lookup_hash::LookupHasher;
pub use masked::Masked;
pub use normalize::normalize;
pub use text_redactor::redact_text;
