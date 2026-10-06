//! Specification tests for T6b-1: `FileService` — keeping an uploaded file and
//! handing it out again.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `upload` judges a file as v2 does (`fixtures/files/v2_upload_cases.json`,
//!   recorded from v2's own upload code), names it with a random UUID, writes
//!   the bytes to `<dir>/<key>` and one row to `files`.
//! - `open_owned` is how the owner reads a file: by key and by who stored it.
//!   `open_in_branch` is how staff read one: by key and by branch.
//! - The directory comes from the setting `files.dir`.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

mod common;
mod files_common;

use std::collections::BTreeSet;

use common::settings;
use files_common::*;

// ----------------------------------------------------------------- settings

#[tokio::test]
async fn the_upload_directory_is_a_setting() {
    // the committed default, relative to where the server is started
    assert_eq!(settings(&[]).files.dir, "uploads");
    // and the environment replaces it, as it does every other setting
    assert_eq!(
        settings(&[("APP_FILES__DIR", "/var/lib/xchange/uploads")])
            .files
            .dir,
        "/var/lib/xchange/uploads"
    );
}

// ------------------------------------------------------------------- upload

#[tokio::test]
async fn every_file_is_judged_as_v2_judges_it() {
    let fx = fixture();
    assert_eq!(fx.cases.len(), 47, "v2_upload_cases.json was cut short");
    assert_eq!(fx.limit, 15 * 1024 * 1024);
    assert_eq!(MAX_UPLOAD_BYTES, fx.limit);
    assert_eq!(
        fx.sentences,
        [NOT_THAT_KIND, UNSUPPORTED, TOO_LARGE],
        "the sentences v2 refused a file with"
    );

    let db = TestDb::start().await;
    let files = db.files();
    let mut accepted = BTreeSet::new();
    let mut wrong = Vec::new();

    for (i, case) in fx.cases.iter().enumerate() {
        let what = format!("case {} '{}'", i + 1, case.label);
        let bytes = case.bytes();
        let mime = case.mime.as_deref().unwrap_or("");
        let before = (db.rows().await.len(), db.names_on_disk());
        let got = files
            .upload(&branch(MAIN), OWNER, mime, &bytes, at(i as i64))
            .await;

        if !case.accepted {
            let want = case
                .error
                .as_deref()
                .expect("a refused case has a sentence");
            match &got {
                Err(pos_error::AppError::Domain(sentence)) if sentence == want => {}
                other => wrong.push(format!("{what}: v2 refused with {want:?}, got {other:?}")),
            }
            // a refused file leaves nothing behind
            if (db.rows().await.len(), db.names_on_disk()) != before {
                wrong.push(format!("{what}: a refused file was stored"));
            }
            continue;
        }

        let key = match got {
            Ok(key) => key,
            Err(e) => {
                wrong.push(format!("{what}: v2 accepted it, got {e:?}"));
                continue;
            }
        };
        if !is_upload_key(&key) {
            wrong.push(format!(
                "{what}: the key {key:?} is not a version-4 UUID in lower case"
            ));
            continue;
        }
        if !accepted.insert(key.clone()) {
            wrong.push(format!("{what}: the key {key} was handed out twice"));
        }
        let want_row = (
            key.clone(),
            MAIN.to_owned(),
            OWNER.to_owned(),
            mime.to_owned(),
            case.sha256.clone().expect("an accepted case has a hash"),
            millis(at(i as i64)),
        );
        if db.row(&key).await.as_ref() != Some(&want_row) {
            wrong.push(format!(
                "{what}: the row is {:?}, expected {want_row:?}",
                db.row(&key).await
            ));
        }
        match std::fs::read(db.dir.join(&key)) {
            Ok(on_disk) if on_disk == bytes => {}
            Ok(on_disk) => wrong.push(format!(
                "{what}: {} bytes on disk, {} were uploaded, or their content differs",
                on_disk.len(),
                bytes.len()
            )),
            Err(e) => wrong.push(format!("{what}: no file named by the key: {e}")),
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(accepted.len(), 11, "files v2 accepted");
    // one row and one file per accepted upload, the file named by its key, nothing else
    let keys: BTreeSet<String> = db.rows().await.into_iter().map(|r| r.0).collect();
    assert_eq!(keys, accepted);
    assert_eq!(db.names_on_disk(), accepted);
    db.finish().await;
}

#[tokio::test]
async fn the_branch_is_judged_before_the_file() {
    let db = TestDb::start().await;
    let files = db.files();
    db.seed_branch("XC-B01", "Second").await;
    let seventy = "a".repeat(70);
    db.seed_branch(&seventy, "Seventy").await;

    // a name that cannot be a branch: v2 does not even look it up
    for bad in [
        "",
        "XC_B",
        "XC B",
        "สาขา",
        "../MAIN",
        "MAIN;",
        &"a".repeat(71),
    ] {
        for (mime, bytes) in [
            ("image/jpeg", JPEG),
            ("image/gif", JPEG),
            ("image/jpeg", PDF),
        ] {
            assert_eq!(
                refusal(files.upload(&branch(bad), OWNER, mime, bytes, at(0)).await),
                BAD_BRANCH,
                "branch {bad:?}, {mime}"
            );
        }
    }
    // a well-formed name that is no branch — whatever is wrong with the file
    for missing in ["XC-B99", "main", "Main", "MAIN-"] {
        for (mime, bytes) in [
            ("image/jpeg", JPEG),
            ("image/gif", JPEG),
            ("image/jpeg", PDF),
        ] {
            assert_eq!(
                refusal(
                    files
                        .upload(&branch(missing), OWNER, mime, bytes, at(0))
                        .await
                ),
                NO_SUCH_BRANCH,
                "branch {missing:?}, {mime}"
            );
        }
    }
    let huge = {
        let mut b = JPEG.to_vec();
        b.resize(MAX_UPLOAD_BYTES + 1, 0);
        b
    };
    assert_eq!(
        refusal(
            files
                .upload(&branch("XC-B99"), OWNER, "image/jpeg", &huge, at(0))
                .await
        ),
        NO_SUCH_BRANCH
    );
    assert_eq!(db.rows().await.len(), 0);
    assert!(db.names_on_disk().is_empty());
    assert!(!db.dir.exists(), "a refused upload created the directory");

    // branches that exist
    let a = files
        .upload(&branch("XC-B01"), OWNER, "image/jpeg", JPEG, at(1))
        .await
        .unwrap();
    let b = files
        .upload(&branch(&seventy), OWNER, "application/pdf", PDF, at(2))
        .await
        .unwrap();
    assert_eq!(db.row(&a).await.unwrap().1, "XC-B01");
    assert_eq!(db.row(&b).await.unwrap().1, seventy);
    db.finish().await;
}

#[tokio::test]
async fn the_same_bytes_twice_are_two_files() {
    let db = TestDb::start().await;
    let files = db.files();
    let first = files
        .upload(&branch(MAIN), OWNER, "image/jpeg", JPEG, at(0))
        .await
        .unwrap();
    let second = files
        .upload(&branch(MAIN), OWNER, "image/jpeg", JPEG, at(0))
        .await
        .unwrap();
    let third = files
        .upload(&branch(MAIN), "employee-7", "image/jpeg", JPEG, at(5))
        .await
        .unwrap();
    assert!(is_upload_key(&first) && is_upload_key(&second) && is_upload_key(&third));
    assert_eq!(BTreeSet::from([&first, &second, &third]).len(), 3);

    let hash = sha256(&db.pool, JPEG).await;
    let rows = db.rows().await;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.4 == hash), "{rows:?}");
    assert_eq!(db.row(&third).await.unwrap().2, "employee-7");
    assert_eq!(db.row(&third).await.unwrap().5, millis(at(5)));
    assert_eq!(
        db.names_on_disk(),
        BTreeSet::from([first, second, third]),
        "one file per upload, named by its key, and nothing else in the directory"
    );
    db.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_stored_file_is_readable_by_the_server_only() {
    use std::os::unix::fs::PermissionsExt;

    let db = TestDb::start().await;
    let key = db
        .files()
        .upload(&branch(MAIN), OWNER, "application/pdf", PDF, at(0))
        .await
        .unwrap();
    let mode = std::fs::metadata(db.dir.join(&key))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "evidence is a passport or a bank slip");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn many_uploads_at_once_are_all_kept() {
    let db = TestDb::start().await;
    let files = db.files();
    let mut tasks = Vec::new();
    for i in 0..64u8 {
        let files = files.clone();
        tasks.push(tokio::spawn(async move {
            // big enough that writing one takes a while
            let mut bytes = JPEG.to_vec();
            bytes.resize(512 * 1024, i);
            let key = files
                .upload(&branch(MAIN), OWNER, "image/jpeg", &bytes, at(i as i64))
                .await
                .expect("an upload");
            (key, bytes)
        }));
    }
    let mut keys = BTreeSet::new();
    for task in tasks {
        let (key, bytes) = tokio::time::timeout(MUST_FINISH, task)
            .await
            .expect("uploads finish")
            .expect("no upload panics");
        assert_eq!(std::fs::read(db.dir.join(&key)).unwrap(), bytes, "{key}");
        let stored = files
            .open_owned(&key, OWNER)
            .await
            .unwrap()
            .expect("it can be opened");
        assert_eq!(stored.bytes, bytes);
        keys.insert(key);
    }
    assert_eq!(keys.len(), 64);
    assert_eq!(db.names_on_disk(), keys);
    assert_eq!(db.rows().await.len(), 64);
    db.finish().await;
}

// --------------------------------------------------------------------- open

#[tokio::test]
async fn the_owner_opens_what_the_owner_stored() {
    let db = TestDb::start().await;
    let files = db.files();
    db.seed_branch("XC-B01", "Second").await;
    let mine = files
        .upload(&branch("XC-B01"), OWNER, "image/jpeg", JPEG, at(3))
        .await
        .unwrap();
    let theirs = files
        .upload(
            &branch("XC-B01"),
            "employee-7",
            "application/pdf",
            PDF,
            at(4),
        )
        .await
        .unwrap();

    let got = files
        .open_owned(&mine, OWNER)
        .await
        .unwrap()
        .expect("the owner's own file");
    assert_eq!(
        got,
        StoredFile {
            key: mine.clone(),
            branch_id: "XC-B01".to_owned(),
            owner_id: OWNER.to_owned(),
            mime: "image/jpeg".to_owned(),
            sha256: sha256(&db.pool, JPEG).await,
            created_at: millis(at(3)),
            bytes: JPEG.to_vec(),
        }
    );
    // whoever stored it reads it, in whatever branch it is
    let got = files
        .open_owned(&theirs, "employee-7")
        .await
        .unwrap()
        .expect("their own file");
    assert_eq!(
        (got.mime.as_str(), got.bytes.as_slice()),
        ("application/pdf", PDF)
    );

    // nobody else does: the answer is "no such file", not an error
    assert_eq!(files.open_owned(&theirs, OWNER).await.unwrap(), None);
    assert_eq!(files.open_owned(&mine, "employee-7").await.unwrap(), None);
    assert_eq!(files.open_owned(&mine, "").await.unwrap(), None);
    assert_eq!(files.open_owned(&mine, "OWNER-1").await.unwrap(), None);
    db.finish().await;
}

#[tokio::test]
async fn staff_open_what_is_in_their_branch() {
    let db = TestDb::start().await;
    let files = db.files();
    db.seed_branch("XC-B01", "Second").await;
    let in_main = files
        .upload(
            &branch(MAIN),
            OWNER,
            "image/png",
            &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1],
            at(1),
        )
        .await
        .unwrap();
    let in_b01 = files
        .upload(
            &branch("XC-B01"),
            "employee-7",
            "application/pdf",
            PDF,
            at(2),
        )
        .await
        .unwrap();

    // whoever stored it
    let got = files
        .open_in_branch(&in_main, &branch(MAIN))
        .await
        .unwrap()
        .expect("a file of MAIN");
    assert_eq!(
        (got.owner_id.as_str(), got.mime.as_str()),
        (OWNER, "image/png")
    );
    assert_eq!(got.bytes.len(), 9);
    let got = files
        .open_in_branch(&in_b01, &branch("XC-B01"))
        .await
        .unwrap()
        .expect("a file of XC-B01");
    assert_eq!(got.bytes, PDF);
    assert_eq!(got.created_at, millis(at(2)));

    // another branch: no such file
    assert_eq!(
        files
            .open_in_branch(&in_main, &branch("XC-B01"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        files.open_in_branch(&in_b01, &branch(MAIN)).await.unwrap(),
        None
    );
    assert_eq!(
        files
            .open_in_branch(&in_b01, &branch("XC-B99"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        files
            .open_in_branch(&in_b01, &branch("xc-b01"))
            .await
            .unwrap(),
        None
    );
    db.finish().await;
}

/// A receipt's key: `receipt-` and 64 lower-case hex digits.
fn receipt_key(digit: char) -> String {
    format!("receipt-{}", digit.to_string().repeat(64))
}

#[tokio::test]
async fn the_two_ways_in_take_the_keys_v2_takes() {
    let db = TestDb::start().await;
    let files = db.files();
    let html = b"<!doctype html><title>receipt</title>";

    // (key, the owner's way takes it, the staff way takes it)
    let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e".to_owned();
    let table: Vec<(String, bool, bool)> = vec![
        (uuid.clone(), true, true),
        (receipt_key('a'), true, true),
        // thirty-six characters of the alphabet, not shaped like a UUID
        (
            "0123456789abcdef0123456789abcdef0123".to_owned(),
            true,
            true,
        ),
        (
            "------------------------------------".to_owned(),
            true,
            true,
        ),
        // 37 to 64 characters: the staff way only
        (format!("{uuid}0"), false, true),
        ("a".repeat(40), false, true),
        ("b".repeat(64), false, true),
        // `receipt-` before 36 to 63 characters: the staff way only
        (format!("receipt-{uuid}"), false, true),
        (format!("receipt-{}", "c".repeat(63)), false, true),
        // a receipt key is hex digits only for the owner; the staff way allows dashes
        (
            format!("receipt-{}-{}", "a".repeat(31), "b".repeat(32)),
            false,
            true,
        ),
        // neither
        ("d".repeat(35), false, false),
        ("e".repeat(65), false, false),
        (format!("receipt-{}", "f".repeat(35)), false, false),
        (format!("receipt-{}", "0".repeat(65)), false, false),
        (uuid.to_uppercase(), false, false),
        (format!("RECEIPT-{}", "a".repeat(64)), false, false),
        (format!("receipt_{}", "a".repeat(64)), false, false),
        (format!("{}g", &uuid[..35]), false, false),
        (format!("{} ", &uuid[..35]), false, false),
        (format!("{}\n", uuid), false, false),
        (format!("{}.", &uuid[..35]), false, false),
        (format!("{}/", &uuid[..35]), false, false),
        (format!("receipt-{}G", "a".repeat(63)), false, false),
    ];
    assert_eq!(table.len(), 23);
    for (key, _, _) in &table {
        // a row and a file for every one of them, so only the key decides
        db.seed_row(key, MAIN, OWNER, "text/html").await;
        if !key.contains('/') && !key.contains('\n') {
            std::fs::create_dir_all(&db.dir).unwrap();
            std::fs::write(db.dir.join(key), html).unwrap();
        }
    }
    for (key, owner_way, staff_way) in &table {
        let got = files.open_owned(key, OWNER).await.unwrap();
        assert_eq!(got.is_some(), *owner_way, "open_owned({key:?})");
        if let Some(file) = got {
            assert_eq!(
                (file.mime.as_str(), file.bytes.as_slice()),
                ("text/html", &html[..])
            );
            assert_eq!(
                file.sha256, "seeded",
                "the row's hash is handed out as stored"
            );
        }
        let got = files.open_in_branch(key, &branch(MAIN)).await.unwrap();
        assert_eq!(got.is_some(), *staff_way, "open_in_branch({key:?})");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_key_never_leads_out_of_the_directory() {
    let db = TestDb::start().await;
    let files = db.files();
    let key = files
        .upload(&branch(MAIN), OWNER, "image/jpeg", JPEG, at(0))
        .await
        .unwrap();
    // a file beside the upload directory, and rows that name it
    std::fs::write(db.dir.parent().unwrap().join("secret"), b"secret").unwrap();
    for bad in [
        "../secret".to_owned(),
        format!("../uploads/{key}"),
        format!("./{key}"),
        format!("{key}/../{key}"),
        db.dir.join(&key).to_string_lossy().into_owned(),
        String::new(),
        ".".to_owned(),
        "..".to_owned(),
    ] {
        db.seed_row(&bad, MAIN, OWNER, "image/jpeg").await;
        assert_eq!(
            files.open_owned(&bad, OWNER).await.unwrap(),
            None,
            "{bad:?}"
        );
        assert_eq!(
            files.open_in_branch(&bad, &branch(MAIN)).await.unwrap(),
            None,
            "{bad:?}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_row_without_its_file_and_a_file_without_its_row_are_no_file() {
    let db = TestDb::start().await;
    let files = db.files();

    // before anything was ever uploaded there is no directory: still "no such file"
    let never = "0f8fad5b-d9cb-469f-a165-70867728950e";
    assert_eq!(files.open_owned(never, OWNER).await.unwrap(), None);
    assert_eq!(
        files.open_in_branch(never, &branch(MAIN)).await.unwrap(),
        None
    );
    db.seed_row(never, MAIN, OWNER, "image/jpeg").await;
    assert_eq!(files.open_owned(never, OWNER).await.unwrap(), None);
    assert_eq!(
        files.open_in_branch(never, &branch(MAIN)).await.unwrap(),
        None
    );
    assert!(!db.dir.exists(), "opening a file created the directory");

    let key = files
        .upload(&branch(MAIN), OWNER, "image/jpeg", JPEG, at(0))
        .await
        .unwrap();
    // the file is gone from the disk
    std::fs::remove_file(db.dir.join(&key)).unwrap();
    assert_eq!(files.open_owned(&key, OWNER).await.unwrap(), None);
    assert_eq!(
        files.open_in_branch(&key, &branch(MAIN)).await.unwrap(),
        None
    );

    // a file nobody recorded
    let stray = "11111111-2222-4333-8444-555555555555";
    std::fs::write(db.dir.join(stray), JPEG).unwrap();
    assert_eq!(files.open_owned(stray, OWNER).await.unwrap(), None);
    assert_eq!(
        files.open_in_branch(stray, &branch(MAIN)).await.unwrap(),
        None
    );
    db.finish().await;
}

#[tokio::test]
async fn what_is_on_disk_is_what_is_handed_out() {
    // the service hands out the stored bytes and the recorded type and hash; it
    // does not judge a file again when it is read
    let db = TestDb::start().await;
    let files = db.files();
    let every_byte: Vec<u8> = (0..=255u8).collect();
    let key = receipt_key('9');
    db.seed_file(&key, MAIN, OWNER, "text/html", &every_byte)
        .await;
    let got = files
        .open_owned(&key, OWNER)
        .await
        .unwrap()
        .expect("a seeded receipt");
    assert_eq!(got.bytes, every_byte);
    assert_eq!(got.mime, "text/html");
    assert_eq!(got.created_at, millis(at(0)));
    // the recorded type is handed out even when the bytes look like something else
    let disguised = receipt_key('7');
    db.seed_file(&disguised, MAIN, OWNER, "text/html", JPEG)
        .await;
    let got = files
        .open_owned(&disguised, OWNER)
        .await
        .unwrap()
        .expect("a seeded file");
    assert_eq!(
        (got.mime.as_str(), got.bytes.as_slice()),
        ("text/html", JPEG)
    );
    let empty = receipt_key('8');
    db.seed_file(&empty, MAIN, OWNER, "text/html", b"").await;
    let got = files
        .open_in_branch(&empty, &branch(MAIN))
        .await
        .unwrap()
        .expect("an empty file");
    assert!(got.bytes.is_empty());
    db.finish().await;
}

// ---------------------------------------------------------------- the table

#[tokio::test]
async fn the_files_table_holds_one_row_per_key_in_a_branch_that_exists() {
    let db = TestDb::start().await;
    let key = "0f8fad5b-d9cb-469f-a165-70867728950e";
    db.seed_row(key, MAIN, OWNER, "image/jpeg").await;

    let again = sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ($1, 'MAIN', 'someone', 'image/png', 'x', now())",
    )
    .bind(key)
    .execute(&db.pool)
    .await;
    assert!(again.is_err(), "the same key was stored twice");

    let nowhere = sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('11111111-2222-4333-8444-555555555555', 'XC-B99', 'someone', 'image/png', 'x', now())",
    )
    .execute(&db.pool)
    .await;
    assert!(
        nowhere.is_err(),
        "a file was stored in a branch that does not exist"
    );

    for column in [
        "key",
        "branch_id",
        "owner_id",
        "mime",
        "sha256",
        "created_at",
    ] {
        let missing = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             SELECT {} FROM (SELECT 'k2'::text AS key, 'MAIN'::text AS branch_id, 'o'::text AS owner_id, \
             'image/png'::text AS mime, 'x'::text AS sha256, now() AS created_at) v",
            ["key", "branch_id", "owner_id", "mime", "sha256", "created_at"]
                .map(|c| if c == column { format!("NULL::{}", if c == "created_at" { "timestamptz" } else { "text" }) } else { c.to_owned() })
                .join(", ")
        )))
        .execute(&db.pool)
        .await;
        assert!(missing.is_err(), "files.{column} took a NULL");
    }
    assert_eq!(db.rows().await.len(), 1);
    db.finish().await;
}
