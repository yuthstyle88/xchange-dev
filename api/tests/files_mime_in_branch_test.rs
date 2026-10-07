//! `FileService::mime_in_branch` answers the stored type of a file only in the
//! branch its record is in — the same rule as `open_in_branch` and v2's
//! `SELECT mime FROM file_records WHERE key=$1 AND branch_id=$2`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! The documents pages reach `mime_in_branch` only with keys of an exchange
//! already found in its own branch, so no test through them can hand it a key
//! whose record is in another branch. These tests ask the service directly.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

// The shared harness re-exports `MAX_UPLOAD_BYTES`, which this file has no use for.
#[allow(unused_imports)]
mod files_common;

use files_common::*;

const SECOND: &str = "XC-B01";

#[tokio::test]
async fn a_record_in_another_branch_has_no_type_here() {
    let db = TestDb::start().await;
    let files = db.files();
    db.seed_branch(SECOND, "Second").await;
    // a record of the second branch, with no bytes on the disk: the type is the record's
    let key = "6f1c2a3b-4d5e-4f60-8a1b-2c3d4e5f6a7b";
    db.seed_row(key, SECOND, "employee-7", "application/pdf")
        .await;

    // in its own branch: the recorded type
    assert_eq!(
        files.mime_in_branch(key, &branch(SECOND)).await.unwrap(),
        Some("application/pdf".to_string())
    );
    // in any other branch: no such file
    for other in [MAIN, "XC-B99", "xc-b01"] {
        assert_eq!(
            files.mime_in_branch(key, &branch(other)).await.unwrap(),
            None,
            "the type of a record of {SECOND} was answered in {other:?}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn an_uploaded_file_has_its_type_only_in_its_own_branch() {
    let db = TestDb::start().await;
    let files = db.files();
    db.seed_branch(SECOND, "Second").await;
    let in_main = files
        .upload(&branch(MAIN), OWNER, "image/jpeg", JPEG, at(1))
        .await
        .unwrap();
    let in_second = files
        .upload(&branch(SECOND), "employee-7", "application/pdf", PDF, at(2))
        .await
        .unwrap();

    // in its own branch: the type the staff way of reading hands out
    let opened: StoredFile = files
        .open_in_branch(&in_main, &branch(MAIN))
        .await
        .unwrap()
        .expect("a file of MAIN");
    assert_eq!(
        files.mime_in_branch(&in_main, &branch(MAIN)).await.unwrap(),
        Some(opened.mime)
    );
    assert_eq!(
        files
            .mime_in_branch(&in_second, &branch(SECOND))
            .await
            .unwrap(),
        Some("application/pdf".to_string())
    );
    assert_eq!(
        files
            .mime_in_branch(&in_main, &branch(SECOND))
            .await
            .unwrap(),
        None,
        "a file of MAIN had a type in {SECOND}"
    );
    assert_eq!(
        files
            .mime_in_branch(&in_second, &branch(MAIN))
            .await
            .unwrap(),
        None,
        "a file of {SECOND} had a type in MAIN"
    );
    db.finish().await;
}
