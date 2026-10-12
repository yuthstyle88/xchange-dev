//! VF-2 (#1387) up-front tests: `FsDocumentStore` behind the `DocumentStore` trait
//! (verification design §5, §9.1 row VF-2; owner S-1 a directory on the v3 host, S-2 sealed with
//! the existing `PiiCipher` and `pii.key`).
//!
//! Each test gets its own temporary directory; the store is the `store/` folder inside it, so a
//! ref that escapes the store would land in the test's own folder where the test can see it.
//! The on-disk object format is `[proposed]` in the ticket, so these tests never parse it: they
//! only list the files the store leaves behind.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::infrastructure::document_store::FsDocumentStore;

const KEY_V1: &str = "v1:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const KEY_V2: &str = "v2:CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";

/// A per-test folder; the store directory is `<root>/store`. Removed on drop.
struct TempRoot {
    root: PathBuf,
}

impl TempRoot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("xc-docstore-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("store")).expect("create temp store dir");
        Self { root }
    }

    fn store_dir(&self) -> PathBuf {
        self.root.join("store")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn store_with(dir: &Path, key_file: &str) -> FsDocumentStore {
    let keys = PiiKeys::parse(key_file).expect("parse test key file");
    FsDocumentStore::new(dir.to_path_buf(), PiiCipher::new(Arc::new(keys)))
}

/// Every regular file under `dir`, recursively, as paths.
fn files_under(dir: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path);
            }
        }
    }
    out
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

const PASSPORT: &[u8] = b"PASSPORT-SCAN P<THADOE<<JOHN<<<<<<<<<<<<<<<<<<<<<<< AA1234567";

#[tokio::test]
async fn put_then_get_round_trips_the_plaintext() {
    let tmp = TempRoot::new();
    let store = store_with(&tmp.store_dir(), KEY_V1);

    let key_id = store.put("doc-1", PASSPORT).await.expect("put");
    assert_eq!(
        key_id, "v1",
        "put must return the key id that sealed the object"
    );

    let got = store.get("doc-1").await.expect("get");
    assert_eq!(got, PASSPORT, "get must return the bytes that were put");
}

#[tokio::test]
async fn delete_then_get_answers_not_found() {
    let tmp = TempRoot::new();
    let store = store_with(&tmp.store_dir(), KEY_V1);

    store.put("doc-del", PASSPORT).await.expect("put");
    store.delete("doc-del").await.expect("delete");

    match store.get("doc-del").await {
        Err(DocumentStoreError::NotFound(_)) => {}
        other => panic!("get after delete must be NotFound, got {other:?}"),
    }
    assert!(
        files_under(&tmp.store_dir()).is_empty(),
        "delete must remove the object file, left: {:?}",
        files_under(&tmp.store_dir())
    );
}

#[tokio::test]
async fn get_of_a_ref_never_put_answers_not_found() {
    let tmp = TempRoot::new();
    let store = store_with(&tmp.store_dir(), KEY_V1);

    match store.get("never-put").await {
        Err(DocumentStoreError::NotFound(_)) => {}
        other => panic!("get of an unknown ref must be NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn a_ref_outside_the_store_directory_is_refused() {
    let tmp = TempRoot::new();
    let store = store_with(&tmp.store_dir(), KEY_V1);

    for bad in ["../x", "/etc/passwd", "a/b", "", ".."] {
        match store.put(bad, PASSPORT).await {
            Err(DocumentStoreError::InvalidRef(_)) => {}
            other => panic!("put({bad:?}) must be InvalidRef, got {other:?}"),
        }
        match store.get(bad).await {
            Err(DocumentStoreError::InvalidRef(_)) => {}
            other => panic!("get({bad:?}) must be InvalidRef, got {other:?}"),
        }
        match store.delete(bad).await {
            Err(DocumentStoreError::InvalidRef(_)) => {}
            other => panic!("delete({bad:?}) must be InvalidRef, got {other:?}"),
        }
    }

    // Nothing was written anywhere: not in the store, not next to it (where `../x` points).
    let written = files_under(&tmp.root);
    assert!(
        written.is_empty(),
        "a refused ref must write nothing, found: {written:?}"
    );
}

#[tokio::test]
async fn the_bytes_on_disk_are_not_the_plaintext() {
    let tmp = TempRoot::new();
    let store = store_with(&tmp.store_dir(), KEY_V1);

    store.put("doc-enc", PASSPORT).await.expect("put");

    let files = files_under(&tmp.store_dir());
    assert!(
        !files.is_empty(),
        "put must write an object under the store directory"
    );
    let needle = &PASSPORT[..24];
    for f in &files {
        let raw = std::fs::read(f).expect("read object file");
        assert!(
            !contains_subslice(&raw, needle),
            "object file {} holds the plaintext",
            f.display()
        );
    }
}

#[tokio::test]
async fn an_object_moved_to_another_ref_fails_to_decrypt() {
    let tmp = TempRoot::new();
    let dir = tmp.store_dir();
    let store = store_with(&dir, KEY_V1);

    store.put("ref-a", PASSPORT).await.expect("put a");
    let after_a = files_under(&dir);
    assert_eq!(
        after_a.len(),
        1,
        "one object for one ref, found {after_a:?}"
    );
    let file_a = after_a.iter().next().unwrap().clone();

    store
        .put("ref-b", b"another document")
        .await
        .expect("put b");
    let after_b = files_under(&dir);
    let new: Vec<_> = after_b.difference(&after_a).cloned().collect();
    assert_eq!(new.len(), 1, "one new object for ref-b, found {new:?}");
    let file_b = &new[0];

    // Put ref-a's sealed object where ref-b's lives: the AAD (the ref) no longer matches.
    std::fs::copy(&file_a, file_b).expect("swap object on disk");

    match store.get("ref-b").await {
        Err(DocumentStoreError::Decrypt(_)) => {}
        Ok(bytes) => panic!(
            "an object sealed for ref-a must not open as ref-b, got {} bytes (plaintext leak: {})",
            bytes.len(),
            bytes == PASSPORT
        ),
        Err(other) => panic!("expected Decrypt for a ref mismatch, got {other:?}"),
    }
    // ref-a itself still opens.
    assert_eq!(store.get("ref-a").await.expect("get a"), PASSPORT);
}

#[tokio::test]
async fn a_key_rotated_in_pii_key_still_reads_old_objects() {
    let tmp = TempRoot::new();
    let dir = tmp.store_dir();

    let old = store_with(&dir, KEY_V1);
    assert_eq!(old.put("doc-old", PASSPORT).await.expect("put old"), "v1");
    drop(old);

    // pii.key after rotation: the old key stays, the new key is the last (current) line.
    let rotated = store_with(&dir, &format!("{KEY_V1}\n{KEY_V2}\n"));
    assert_eq!(
        rotated
            .get("doc-old")
            .await
            .expect("old object after rotation"),
        PASSPORT,
        "an object sealed with v1 must still open after v2 becomes current"
    );

    let new_id = rotated
        .put("doc-new", b"sealed with the new key")
        .await
        .expect("put new");
    assert_eq!(new_id, "v2", "new objects are sealed with the current key");
    assert_eq!(
        rotated.get("doc-new").await.expect("get new"),
        b"sealed with the new key"
    );

    // A store that has lost v2 cannot open the v2 object: the key id really was recorded.
    let v1_only = store_with(&dir, KEY_V1);
    match v1_only.get("doc-new").await {
        Err(DocumentStoreError::Decrypt(_)) => {}
        other => panic!("a v2 object read with only v1 must be Decrypt, got {other:?}"),
    }
}

#[tokio::test]
async fn the_same_ref_put_twice_replaces_and_leaves_no_temp_file() {
    let tmp = TempRoot::new();
    let dir = tmp.store_dir();
    let store = store_with(&dir, KEY_V1);

    store
        .put("doc-twice", b"first version")
        .await
        .expect("put 1");
    store
        .put("doc-twice", b"second version")
        .await
        .expect("put 2");

    assert_eq!(
        store.get("doc-twice").await.expect("get"),
        b"second version",
        "the second put must replace the first"
    );
    let files = files_under(&dir);
    assert_eq!(
        files.len(),
        1,
        "one ref = one file; no temp file may be left behind, found {files:?}"
    );
}
