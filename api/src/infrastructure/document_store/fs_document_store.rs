//! `FsDocumentStore`: a directory on the v3 host outside `files.dir` (owner S-1), each object
//! sealed with the existing `PiiCipher` and `pii.key` (owner S-2), AAD = the object ref.
//!
//! One ref is one file, `<dir>/<ref>`. Its content is the sealing key's id and the sealed bytes:
//! `[key id length: 1 byte][key id][nonce + ciphertext + tag]`. A write goes to a temporary name
//! first and is renamed into place, so a reader never sees half an object.

use std::path::{Path, PathBuf};

use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::pii::{PiiCipher, PiiError, Sealed};

/// `futures::future::BoxFuture`, spelled without the `futures` crate (the `api` crate has none).
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Prefix of the temporary files a write leaves while it runs; refs may not start with `.`.
const TEMP_PREFIX: &str = ".tmp-";

/// Filesystem document store.
pub struct FsDocumentStore {
    dir: PathBuf,
    cipher: PiiCipher,
}

impl FsDocumentStore {
    /// A store rooted at `dir`, sealing with `cipher`.
    pub fn new(dir: impl Into<PathBuf>, cipher: PiiCipher) -> Self {
        Self {
            dir: dir.into(),
            cipher,
        }
    }

    /// The object file of `doc_ref`, or `InvalidRef` when the ref could leave the store directory:
    /// empty, absolute, `.`/`..`, a path separator, a NUL, or a leading `.` (the temporary files').
    fn path_of(&self, doc_ref: &str) -> Result<PathBuf, DocumentStoreError> {
        let refused = doc_ref.is_empty()
            || doc_ref.starts_with('.')
            || doc_ref.contains("..")
            || doc_ref.contains(['/', '\\', '\0'])
            || Path::new(doc_ref).is_absolute();
        if refused {
            return Err(DocumentStoreError::InvalidRef(doc_ref.to_owned()));
        }
        Ok(self.dir.join(doc_ref))
    }
}

impl DocumentStore for FsDocumentStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>> {
        Box::pin(async move {
            let path = self.path_of(doc_ref)?;
            let sealed = self.cipher.encrypt(doc_ref.as_bytes(), bytes);
            let object = encode(&sealed)?;
            let dir = self.dir.clone();
            tokio::task::spawn_blocking(move || write_file_blocking(&dir, &path, &object))
                .await
                .map_err(|e| DocumentStoreError::Io(format!("write task failed: {e}")))?
                .map_err(|e| DocumentStoreError::Io(format!("write {doc_ref}: {e}")))?;
            Ok(sealed.key_id)
        })
    }

    fn get<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<Vec<u8>, DocumentStoreError>> {
        Box::pin(async move {
            let path = self.path_of(doc_ref)?;
            let object = tokio::task::spawn_blocking(move || read_file_blocking(&path))
                .await
                .map_err(|e| DocumentStoreError::Io(format!("read task failed: {e}")))?
                .map_err(|e| DocumentStoreError::Io(format!("read {doc_ref}: {e}")))?
                .ok_or_else(|| DocumentStoreError::NotFound(doc_ref.to_owned()))?;
            let sealed = decode(&object)?;
            Ok(self.cipher.decrypt(doc_ref.as_bytes(), &sealed)?)
        })
    }

    fn delete<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<(), DocumentStoreError>> {
        Box::pin(async move {
            let path = self.path_of(doc_ref)?;
            tokio::task::spawn_blocking(move || remove_file_blocking(&path))
                .await
                .map_err(|e| DocumentStoreError::Io(format!("delete task failed: {e}")))?
                .map_err(|e| DocumentStoreError::Io(format!("delete {doc_ref}: {e}")))
        })
    }
}

/// `[key id length][key id][sealed bytes]`.
fn encode(sealed: &Sealed) -> Result<Vec<u8>, DocumentStoreError> {
    let id = sealed.key_id.as_bytes();
    let len = u8::try_from(id.len()).map_err(|_| {
        DocumentStoreError::Io(format!("key id longer than 255 bytes: {}", sealed.key_id))
    })?;
    let mut object = Vec::with_capacity(1 + id.len() + sealed.bytes.len());
    object.push(len);
    object.extend_from_slice(id);
    object.extend_from_slice(&sealed.bytes);
    Ok(object)
}

/// The inverse of [`encode`]; a malformed object is a decrypt failure, like a tampered one.
fn decode(object: &[u8]) -> Result<Sealed, DocumentStoreError> {
    let (&len, rest) = object.split_first().ok_or(PiiError::Decrypt)?;
    let len = usize::from(len);
    if rest.len() < len {
        return Err(PiiError::Decrypt.into());
    }
    let (id, bytes) = rest.split_at(len);
    let key_id = std::str::from_utf8(id).map_err(|_| PiiError::Decrypt)?;
    Ok(Sealed {
        key_id: key_id.to_owned(),
        bytes: bytes.to_vec(),
    })
}

/// Writes `bytes` to `target`: a temporary name of its own in `dir` first, then a rename, so a
/// reader never sees half an object and a failed write leaves nothing (the pattern of
/// `files/file_service.rs` `write_file_blocking`). Creates the directory when missing.
fn write_file_blocking(dir: &Path, target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;

    let temp_path = dir.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4()));

    let write_res = (|| -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options.open(&temp_path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&temp_path, bytes)?;
        }
        std::fs::rename(&temp_path, target)?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    Ok(())
}

/// The object's bytes, or `None` when there is none.
fn read_file_blocking(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Removes the object; one that is already gone is fine (delete is idempotent).
fn remove_file_blocking(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
