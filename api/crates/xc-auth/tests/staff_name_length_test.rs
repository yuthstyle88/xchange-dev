//! The staff door's 40 limit counts a name as v2 did: in UTF-16 units, after the name is
//! trimmed and lower-cased. A name within 40 units is an attempt like any other — the
//! address and name counters are raised before the password is looked at; a longer one is
//! refused as wrong credentials with nothing counted.
//!
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//! No PostgreSQL here: the repository is a test double that counts attempts per key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde_json::json;
use sha2::{Digest, Sha256};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::owner::Owner;
use xc_auth::domain::account::repositories::account_repository::{AccountRepository, Purged};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;

const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
const IP: &str = "10.0.0.7";
const PASSWORD: &str = "any password 01";

/// One Thai letter: 3 bytes in UTF-8, 1 UTF-16 unit, 1 `char`.
const THAI: &str = "ก";
/// One emoji outside the BMP: 4 bytes in UTF-8, 2 UTF-16 units, 1 `char`.
const EMOJI: &str = "😀";
/// `İ` (U+0130) lower-cases to `i` + U+0307: 1 unit before, 2 units after — in Rust and in JS.
const DOTTED_I: &str = "İ";

/// No owner and no employee: every name is unknown, so every counted attempt ends in WRONG
/// until a counter passes its limit. `increment_attempt` keeps a real count per key and
/// records every key it was asked for, in order.
#[derive(Default, Clone)]
struct CountingRepository(Arc<Mutex<Counts>>);

#[derive(Default)]
struct Counts {
    per_key: HashMap<String, i32>,
    asked: Vec<String>,
}

impl CountingRepository {
    fn asked(&self) -> Vec<String> {
        self.0.lock().expect("the test double's lock").asked.clone()
    }
}

impl AccountRepository for CountingRepository {
    async fn find_owner_by_username(&self, _username: &str) -> AppResult<Option<(Owner, String)>> {
        Ok(None)
    }

    async fn find_owner_session(
        &self,
        _token_hash: &str,
    ) -> AppResult<Option<(Owner, i32, DateTime<Utc>)>> {
        unreachable!("not used by these tests")
    }

    async fn create_owner_session(
        &self,
        _token_hash: &str,
        _owner_id: &str,
        _version: i32,
        _expires: DateTime<Utc>,
    ) -> AppResult<()> {
        unreachable!("no name in these tests signs in")
    }

    async fn delete_owner_session(&self, _token_hash: &str) -> AppResult<()> {
        unreachable!("not used by these tests")
    }

    async fn find_employee_by_username(
        &self,
        _username: &str,
    ) -> AppResult<Option<(Employee, String)>> {
        Ok(None)
    }

    async fn find_employee_by_id(&self, _id: &str) -> AppResult<Option<Employee>> {
        unreachable!("not used by these tests")
    }

    async fn find_employee_session(
        &self,
        _token_hash: &str,
    ) -> AppResult<Option<(Employee, i32, DateTime<Utc>)>> {
        unreachable!("not used by these tests")
    }

    async fn create_employee_session(
        &self,
        _token_hash: &str,
        _employee_id: &str,
        _version: i32,
        _expires: DateTime<Utc>,
    ) -> AppResult<()> {
        unreachable!("no name in these tests signs in")
    }

    async fn delete_employee_session(&self, _token_hash: &str) -> AppResult<()> {
        unreachable!("not used by these tests")
    }

    async fn list_employees(&self) -> AppResult<Vec<Employee>> {
        unreachable!("not used by these tests")
    }

    async fn create_employee(
        &self,
        _employee: &Employee,
        _password_hash: &str,
        _actor: &str,
    ) -> AppResult<()> {
        unreachable!("not used by these tests")
    }

    async fn reset_employee_password(
        &self,
        _id: &str,
        _password_hash: &str,
        _now: DateTime<Utc>,
        _actor: &str,
    ) -> AppResult<()> {
        unreachable!("not used by these tests")
    }

    async fn disable_employee(
        &self,
        _id: &str,
        _now: DateTime<Utc>,
        _actor: &str,
    ) -> AppResult<()> {
        unreachable!("not used by these tests")
    }

    async fn change_employee_password(
        &self,
        _id: &str,
        _expected_version: i32,
        _password_hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unreachable!("not used by these tests")
    }

    async fn active_employees(&self, _branch_id: &str) -> AppResult<Vec<Employee>> {
        unreachable!("not used by these tests")
    }

    async fn create_owner(
        &self,
        _id: &str,
        _username: &str,
        _name: &str,
        _password_hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Owner> {
        unreachable!("not used by these tests")
    }

    async fn record_employee_login_audit(
        &self,
        _employee_id: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unreachable!("no name in these tests signs in")
    }

    async fn purge_expired(
        &self,
        _now: DateTime<Utc>,
        _current_window: DateTime<Utc>,
    ) -> AppResult<Purged> {
        unreachable!("not used by these tests")
    }

    async fn increment_attempt(&self, key: &str, _window_start: DateTime<Utc>) -> AppResult<i32> {
        let mut counts = self.0.lock().expect("the test double's lock");
        counts.asked.push(key.to_owned());
        let count = counts.per_key.entry(key.to_owned()).or_insert(0);
        *count += 1;
        Ok(*count)
    }
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 2026-10-07 03:00:00 UTC: every attempt of a test falls in one 15-minute window.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap()
}

fn world() -> (CountingRepository, AccountService<CountingRepository>) {
    let repo = CountingRepository::default();
    let service = AccountService::new(Arc::new(repo.clone()), PasswordHasher::with_cost(8, 1, 1));
    (repo, service)
}

fn sentence(answer: AppResult<impl std::fmt::Debug>) -> String {
    match answer {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected a refusal with a sentence, got {other:?}"),
    }
}

/// One attempt at the staff door with `username` as sent; the refusal's sentence.
async fn staff_attempt(service: &AccountService<CountingRepository>, username: &str) -> String {
    sentence(
        service
            .login_employee(
                &json!({ "username": username, "password": PASSWORD }),
                IP,
                now(),
            )
            .await,
    )
}

/// The keys a counted attempt for `name` (already trimmed and lower-cased) raises, in order.
fn counted_keys(name: &str) -> Vec<String> {
    vec![
        sha256_hex(&format!("IP:{IP}")),
        sha256_hex(&format!("USER:{name}")),
    ]
}

/// Sends `username` once; asserts WRONG, and that the attempt was counted (or not).
async fn assert_one_attempt(username: &str, counted_as: Option<&str>) {
    let (repo, service) = world();
    assert_eq!(staff_attempt(&service, username).await, WRONG);
    let expected = counted_as.map(counted_keys).unwrap_or_default();
    assert_eq!(
        repo.asked(),
        expected,
        "username of {} bytes, {} UTF-16 units: {}",
        username.len(),
        username.encode_utf16().count(),
        if counted_as.is_some() {
            "within 40 units, so the address and the name are counted"
        } else {
            "over 40 units, so refused with nothing counted"
        }
    );
}

// ── ASCII: bytes and units agree, so this pins what already holds ─────────

#[tokio::test]
async fn an_ascii_name_of_40_is_counted() {
    let name = "a".repeat(40);
    assert_one_attempt(&name, Some(&name)).await;
}

#[tokio::test]
async fn an_ascii_name_of_41_is_refused_with_nothing_counted() {
    assert_one_attempt(&"a".repeat(41), None).await;
}

// ── Thai: 1 unit, 3 bytes per letter ─────────────────────────────────────

#[tokio::test]
async fn a_thai_name_of_14_letters_is_counted() {
    let name = THAI.repeat(14);
    assert_one_attempt(&name, Some(&name)).await;
}

#[tokio::test]
async fn a_thai_name_of_40_letters_is_counted() {
    let name = THAI.repeat(40);
    assert_one_attempt(&name, Some(&name)).await;
}

#[tokio::test]
async fn a_thai_name_of_41_letters_is_refused_with_nothing_counted() {
    assert_one_attempt(&THAI.repeat(41), None).await;
}

// ── Emoji: 2 units, 4 bytes, 1 char each ─────────────────────────────────

#[tokio::test]
async fn twenty_emoji_are_40_units_and_counted() {
    let name = EMOJI.repeat(20);
    assert_one_attempt(&name, Some(&name)).await;
}

#[tokio::test]
async fn twenty_one_emoji_are_42_units_and_refused_with_nothing_counted() {
    // 21 chars: a count of `char`s would let this through.
    assert_one_attempt(&EMOJI.repeat(21), None).await;
}

// ── The name is measured after trimming and lower-casing, as v2 ──────────

#[tokio::test]
async fn spaces_around_a_thai_name_of_40_do_not_count() {
    let name = THAI.repeat(40);
    assert_one_attempt(&format!("   {name}   "), Some(&name)).await;
}

#[tokio::test]
async fn a_name_is_measured_after_lower_casing() {
    // 20 × İ is 20 units as sent and 40 units lower-cased: counted.
    let sent = DOTTED_I.repeat(20);
    let lowered = sent.to_lowercase();
    assert_eq!(lowered.encode_utf16().count(), 40);
    assert_one_attempt(&sent, Some(&lowered)).await;
}

#[tokio::test]
async fn a_name_over_40_units_once_lower_cased_is_refused_with_nothing_counted() {
    // 21 × İ is 21 units as sent and 42 units lower-cased: refused before counting.
    assert_one_attempt(&DOTTED_I.repeat(21), None).await;
}

// ── What the count changes: the limit is reached as in v2 ────────────────

#[tokio::test]
async fn the_eleventh_attempt_with_one_thai_name_is_refused_with_the_limit() {
    let (_repo, service) = world();
    let name = THAI.repeat(14);
    for attempt in 1..=10 {
        assert_eq!(
            staff_attempt(&service, &name).await,
            WRONG,
            "attempt {attempt} of 10 is answered as wrong credentials"
        );
    }
    assert_eq!(
        staff_attempt(&service, &name).await,
        LIMIT,
        "the 11th attempt for one name within the window is over the name limit of 10"
    );
}

#[tokio::test]
async fn thai_names_count_toward_the_address_limit() {
    let (_repo, service) = world();
    // 30 different Thai names of 14 letters from one address: none passes its own
    // name limit, and each raises the address counter.
    for n in 0..30u32 {
        let name = format!("{}{}", THAI.repeat(13), char::from_u32(0x0E01 + n).unwrap());
        assert_eq!(staff_attempt(&service, &name).await, WRONG, "attempt {n}");
    }
    assert_eq!(
        staff_attempt(&service, "anyone").await,
        LIMIT,
        "the 31st attempt from one address within the window is over the address limit of 30"
    );
}

#[tokio::test]
async fn the_shared_door_counts_a_thai_name_the_same_way() {
    // `/api/auth` (`login`) looks for an owner first and, finding none, takes the staff path.
    let (repo, service) = world();
    let name = THAI.repeat(14);
    let answer = service
        .login(
            &json!({ "username": name, "password": PASSWORD }),
            IP,
            now(),
        )
        .await;
    assert_eq!(sentence(answer), WRONG);
    assert_eq!(repo.asked(), counted_keys(&name));
}
