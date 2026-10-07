//! File service.

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

/// 15 MB as the old server counts it.
pub const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;

// The refusal sentences from v2
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const UNSUPPORTED: &str = "ชนิดไฟล์ไม่รองรับ";
pub const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
pub const NOT_THAT_KIND: &str = "ชนิดไฟล์ไม่ตรงกับเนื้อหา";

/// A stored file with everything recorded about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub key: String,
    pub branch_id: String,
    pub owner_id: String,
    pub mime: String,
    pub sha256: String,
    pub created_at: DateTime<Utc>,
    pub bytes: Vec<u8>,
}

/// What is recorded of an uploaded file, without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub key: String,
    pub mime: String,
    pub sha256: String,
}

#[derive(Clone)]
pub struct FileService {
    pool: PgPool,
    dir: PathBuf,
}

impl FileService {
    pub fn new(pool: PgPool, dir: impl Into<PathBuf>) -> Self {
        Self {
            pool,
            dir: dir.into(),
        }
    }

    /// Judges and keeps one file. Returns its key.
    pub async fn upload(
        &self,
        branch: &BranchId,
        owner_id: &str,
        mime: &str,
        bytes: &[u8],
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        // 1. branch is 1 to 70 characters of A-Z a-z 0-9 -
        let branch_str = branch.as_str();
        if branch_str.is_empty()
            || branch_str.len() > 70
            || !branch_str
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(AppError::Domain(BAD_BRANCH.to_string()));
        }

        // 2. a row of branches has exactly that id (case-sensitive)
        let branch_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                .bind(branch_str)
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;

        if !branch_exists {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        // 3. mime is exactly one of image/jpeg, image/png, image/webp, application/pdf
        if !matches!(
            mime,
            "image/jpeg" | "image/png" | "image/webp" | "application/pdf"
        ) {
            return Err(AppError::Domain(UNSUPPORTED.to_string()));
        }

        // 4. bytes.len() <= MAX_UPLOAD_BYTES
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(AppError::Domain(TOO_LARGE.to_string()));
        }

        // 5. the first bytes are what mime says
        let matches_content = match mime {
            "image/jpeg" => bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8,
            "image/png" => {
                bytes.len() >= 8 && bytes[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
            }
            "application/pdf" => bytes.len() >= 5 && &bytes[0..5] == b"%PDF-",
            "image/webp" => {
                bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
            }
            _ => false,
        };

        if !matches_content {
            return Err(AppError::Domain(NOT_THAT_KIND.to_string()));
        }

        // 1. The key is a new random UUID, version 4, lower case with dashes
        let key = uuid::Uuid::new_v4().to_string();

        // 2. The hash is the SHA-256 of the bytes in lower-case hex
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha256_hex = format!("{:x}", hasher.finalize());

        // 3. The bytes are written to <dir>/<key>
        // The directory is created when missing, parents included.
        // Write to a temporary name inside <dir> that no other upload can share, then rename.
        // On Unix the file's mode is 0600.
        self.write_file(&key, bytes)?;
        let target_path = self.dir.join(&key);

        // 4. One row: key, branch_id = the branch, owner_id, mime as given, sha256, created_at = now cut to milliseconds.
        let stored_created_at = chrono::Utc
            .timestamp_millis_opt(now.timestamp_millis())
            .single()
            .unwrap_or(now);

        let db_res = sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&key)
        .bind(branch_str)
        .bind(owner_id)
        .bind(mime)
        .bind(&sha256_hex)
        .bind(stored_created_at)
        .execute(&self.pool)
        .await;

        if let Err(e) = db_res {
            let _ = std::fs::remove_file(&target_path);
            return Err(AppError::Database(e));
        }

        Ok(key)
    }

    /// The owner's way of reading: by key and by who stored it.
    pub async fn open_owned(&self, key: &str, owner_id: &str) -> AppResult<Option<StoredFile>> {
        if !is_valid_owned_key(key) {
            return Ok(None);
        }

        let row: Option<(String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT key, branch_id, owner_id, mime, sha256, created_at \
                 FROM files WHERE key = $1 AND owner_id = $2",
        )
        .bind(key)
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let Some((key, branch_id, owner_id, mime, sha256, created_at)) = row else {
            return Ok(None);
        };

        let path = self.dir.join(&key);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Internal(e.to_string())),
        };

        Ok(Some(StoredFile {
            key,
            branch_id,
            owner_id,
            mime,
            sha256,
            created_at,
            bytes,
        }))
    }

    /// The staff way of reading: by key and by branch.
    pub async fn open_in_branch(
        &self,
        key: &str,
        branch: &BranchId,
    ) -> AppResult<Option<StoredFile>> {
        if !is_valid_branch_key(key) {
            return Ok(None);
        }

        let row: Option<(String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT key, branch_id, owner_id, mime, sha256, created_at \
                 FROM files WHERE key = $1 AND branch_id = $2",
        )
        .bind(key)
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let Some((key, branch_id, owner_id, mime, sha256, created_at)) = row else {
            return Ok(None);
        };

        let path = self.dir.join(&key);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Internal(e.to_string())),
        };

        Ok(Some(StoredFile {
            key,
            branch_id,
            owner_id,
            mime,
            sha256,
            created_at,
            bytes,
        }))
    }

    /// The stored type of a file's record in a branch, whatever is on the disk:
    /// what a listing needs to know of a file, without reading its bytes.
    pub async fn mime_in_branch(&self, key: &str, branch: &BranchId) -> AppResult<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT mime FROM files WHERE key = $1 AND branch_id = $2")
                .bind(key)
                .bind(branch.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(row.map(|(mime,)| mime))
    }

    /// Writes `bytes` to `<dir>/<key>`: a temporary name of its own first, then a
    /// rename, so a reader never sees half a file. Creates the directory when missing.
    fn write_file(&self, key: &str, bytes: &[u8]) -> AppResult<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| AppError::Internal(e.to_string()))?;

        let temp_path = self.dir.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let target_path = self.dir.join(key);

        let write_res = (|| -> Result<(), std::io::Error> {
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
            std::fs::rename(&temp_path, &target_path)?;
            Ok(())
        })();

        if let Err(e) = write_res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(AppError::Internal(e.to_string()));
        }
        Ok(())
    }

    /// The record of an upload this owner stored in this branch, whatever is on
    /// the disk: what a save needs to know of a file a form names.
    pub async fn attachment(
        &self,
        key: &str,
        owner_id: &str,
        branch: &BranchId,
    ) -> AppResult<Option<Attachment>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT key, mime, sha256 FROM files \
             WHERE key = $1 AND owner_id = $2 AND branch_id = $3",
        )
        .bind(key)
        .bind(owner_id)
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(row.map(|(key, mime, sha256)| Attachment { key, mime, sha256 }))
    }

    /// Whether the bytes of a key are on the disk.
    pub fn on_disk(&self, key: &str) -> bool {
        self.dir.join(key).is_file()
    }

    /// Keeps a receipt and returns its key, `receipt-<SHA-256 of the text>`.
    ///
    /// The row is written with the caller's transaction, so it exists exactly
    /// when the exchange it belongs to does. The bytes are written first; a
    /// receipt left on the disk by a save that then failed is named by its
    /// content and is found again, or never asked for.
    pub async fn store_receipt_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        owner_id: &str,
        html: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        let sha256_hex = format!("{:x}", Sha256::digest(html.as_bytes()));
        let key = format!("receipt-{sha256_hex}");
        if !self.on_disk(&key) {
            self.write_file(&key, html.as_bytes())?;
        }
        let created_at = chrono::Utc
            .timestamp_millis_opt(now.timestamp_millis())
            .single()
            .unwrap_or(now);
        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, $3, 'text/html', $4, $5) ON CONFLICT (key) DO NOTHING",
        )
        .bind(&key)
        .bind(branch.as_str())
        .bind(owner_id)
        .bind(&sha256_hex)
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(key)
    }
}

/// The key must be either exactly 36 characters each one of 0-9 a-f -,
/// or receipt- followed by exactly 64 characters each one of 0-9 a-f.
fn is_valid_owned_key(key: &str) -> bool {
    if key.len() == 36 {
        key.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
    } else if key.len() == 72 && key.starts_with("receipt-") {
        key[8..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    } else {
        false
    }
}

/// The key must be: an optional receipt-, then 36 to 64 characters each one of 0-9 a-f -.
fn is_valid_branch_key(key: &str) -> bool {
    let rest = key.strip_prefix("receipt-").unwrap_or(key);
    (36..=64).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}
