//! PII core: identity types, normalisation, keys, AES-256-GCM cipher, keyed lookup hasher, and masking.

pub mod cipher;
pub mod error;
pub mod identity_type;
pub mod key_file;
pub mod lookup_hash;
pub mod masked;
pub mod normalize;

pub use cipher::{PiiCipher, Sealed};
pub use error::PiiError;
pub use identity_type::{IdentityType, PiiKind};
pub use key_file::PiiKeys;
pub use lookup_hash::LookupHasher;
pub use masked::Masked;
pub use normalize::normalize;
