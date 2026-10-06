//! What `AccountService` answers when the database fails, and what the spare
//! verification of an unknown name costs (ticket #98).
//!
//! No PostgreSQL here: the repository is a test double that can be told to fail.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde_json::json;
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::owner::Owner;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::account::repositories::account_repository::AccountRepository;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;

const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
const OWNER_PASSWORD: &str = "owner password 01";
const STAFF_PASSWORD: &str = "staff password 01";
const UNKNOWN_TOKEN: &str = "00000000000000000000000000000000000000000000000000000000000000ff";

#[derive(Default)]
struct State {
    owner: Option<(Owner, String)>,
    employee: Option<(Employee, String)>,
    /// token hash → (version, expires)
    owner_sessions: HashMap<String, (i32, DateTime<Utc>)>,
    employee_sessions: HashMap<String, (i32, DateTime<Utc>)>,
    fail_owner_delete: bool,
    fail_employee_delete: bool,
    fail_owner_session_read: bool,
    fail_employee_session_read: bool,
    owner_deletes: u32,
    employee_deletes: u32,
    employee_session_reads: u32,
}

/// An `AccountRepository` in memory: one owner, one employee, their sessions,
/// and switches that make single calls fail as the database would.
#[derive(Default, Clone)]
struct FakeRepository(Arc<Mutex<State>>);

fn database_failure() -> AppError {
    AppError::Database(sqlx::Error::PoolClosed)
}

impl FakeRepository {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().expect("the test double's lock")
    }
}

impl AccountRepository for FakeRepository {
    async fn find_owner_by_username(&self, username: &str) -> AppResult<Option<(Owner, String)>> {
        Ok(self
            .state()
            .owner
            .clone()
            .filter(|(owner, _)| owner.username == username))
    }

    async fn find_owner_session(
        &self,
        token_hash: &str,
    ) -> AppResult<Option<(Owner, i32, DateTime<Utc>)>> {
        let state = self.state();
        if state.fail_owner_session_read {
            return Err(database_failure());
        }
        Ok(state
            .owner_sessions
            .get(token_hash)
            .and_then(|(version, expires)| {
                state
                    .owner
                    .clone()
                    .map(|(owner, _)| (owner, *version, *expires))
            }))
    }

    async fn create_owner_session(
        &self,
        token_hash: &str,
        _owner_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> AppResult<()> {
        self.state()
            .owner_sessions
            .insert(token_hash.to_owned(), (version, expires));
        Ok(())
    }

    async fn delete_owner_session(&self, token_hash: &str) -> AppResult<()> {
        let mut state = self.state();
        state.owner_deletes += 1;
        if state.fail_owner_delete {
            return Err(database_failure());
        }
        state.owner_sessions.remove(token_hash);
        Ok(())
    }

    async fn find_employee_by_username(
        &self,
        username: &str,
    ) -> AppResult<Option<(Employee, String)>> {
        Ok(self
            .state()
            .employee
            .clone()
            .filter(|(employee, _)| employee.username == username))
    }

    async fn find_employee_by_id(&self, id: &str) -> AppResult<Option<Employee>> {
        Ok(self
            .state()
            .employee
            .clone()
            .map(|(employee, _)| employee)
            .filter(|employee| employee.id == id))
    }

    async fn find_employee_session(
        &self,
        token_hash: &str,
    ) -> AppResult<Option<(Employee, i32, DateTime<Utc>)>> {
        let mut state = self.state();
        state.employee_session_reads += 1;
        if state.fail_employee_session_read {
            return Err(database_failure());
        }
        Ok(state
            .employee_sessions
            .get(token_hash)
            .and_then(|(version, expires)| {
                state
                    .employee
                    .clone()
                    .map(|(employee, _)| (employee, *version, *expires))
            }))
    }

    async fn create_employee_session(
        &self,
        token_hash: &str,
        _employee_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> AppResult<()> {
        self.state()
            .employee_sessions
            .insert(token_hash.to_owned(), (version, expires));
        Ok(())
    }

    async fn delete_employee_session(&self, token_hash: &str) -> AppResult<()> {
        let mut state = self.state();
        state.employee_deletes += 1;
        if state.fail_employee_delete {
            return Err(database_failure());
        }
        state.employee_sessions.remove(token_hash);
        Ok(())
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
        Ok(())
    }

    async fn purge_expired(
        &self,
        _now: DateTime<Utc>,
        _current_window: DateTime<Utc>,
    ) -> AppResult<xc_auth::domain::account::repositories::account_repository::Purged> {
        unreachable!("not used by these tests")
    }

    async fn increment_attempt(&self, _key: &str, _window_start: DateTime<Utc>) -> AppResult<i32> {
        Ok(1)
    }
}

fn cheap_hasher() -> PasswordHasher {
    PasswordHasher::with_cost(8, 1, 1)
}

/// `seconds` after 2026-10-07 03:00:00 UTC.
fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

struct World {
    repo: Arc<FakeRepository>,
    service: AccountService<FakeRepository>,
}

/// One active owner `boss` and one active employee `anna` whose first password is already changed.
fn world() -> World {
    let hasher = cheap_hasher();
    let repo = Arc::new(FakeRepository::default());
    {
        let mut state = repo.state();
        state.owner = Some((
            Owner {
                id: "owner-1".to_owned(),
                username: "boss".to_owned(),
                name: "boss".to_owned(),
                active: true,
                version: 1,
                created_at: at(0),
            },
            hasher
                .hash(OWNER_PASSWORD)
                .expect("hash the owner's password"),
        ));
        state.employee = Some((
            Employee {
                id: "employee-1".to_owned(),
                username: "anna".to_owned(),
                name: "Anna".to_owned(),
                role: "FRONT".to_owned(),
                branch_id: "HQ".to_owned(),
                active: true,
                must_change: false,
                version: 1,
                created_at: at(0),
            },
            hasher
                .hash(STAFF_PASSWORD)
                .expect("hash the employee's password"),
        ));
    }
    let service = AccountService::new(repo.clone(), hasher);
    World { repo, service }
}

impl World {
    async fn owner_token(&self) -> String {
        self.service
            .login(
                &json!({"username": "boss", "password": OWNER_PASSWORD}),
                "10.0.0.1",
                at(0),
            )
            .await
            .expect("the owner signs in")
            .token
    }

    async fn staff_token(&self) -> String {
        self.service
            .login_employee(
                &json!({"username": "anna", "password": STAFF_PASSWORD}),
                "10.0.0.1",
                at(0),
            )
            .await
            .expect("the employee signs in")
            .token
    }

    fn sessions(&self) -> (usize, usize) {
        let state = self.repo.state();
        (state.owner_sessions.len(), state.employee_sessions.len())
    }
}

fn is_database_failure<T: std::fmt::Debug>(answer: &AppResult<T>) -> bool {
    matches!(answer, Err(AppError::Database(_)))
}

/// The `m=…,t=…,p=…` field of an Argon2id PHC string.
fn cost_of(phc: &str) -> String {
    let fields: Vec<&str> = phc.split('$').collect();
    assert!(
        fields.len() == 6 && fields[1] == "argon2id" && fields[2] == "v=19",
        "not an Argon2id PHC string: {phc:?}"
    );
    fields[3].to_owned()
}

// ---------------------------------------------------------------- logout

#[tokio::test]
async fn logout_reports_a_failed_staff_delete() {
    let w = world();
    let staff = w.staff_token().await;
    w.repo.state().fail_employee_delete = true;

    let answer = w.service.logout("", &staff).await;

    assert!(
        is_database_failure(&answer),
        "a sign-out whose delete failed must not answer success, got {answer:?}"
    );
    assert_eq!(w.sessions(), (0, 1), "the session row is still there");
}

#[tokio::test]
async fn logout_reports_a_failed_owner_delete() {
    let w = world();
    let owner = w.owner_token().await;
    w.repo.state().fail_owner_delete = true;

    let answer = w.service.logout(&owner, "").await;

    assert!(
        is_database_failure(&answer),
        "a sign-out whose delete failed must not answer success, got {answer:?}"
    );
    assert_eq!(w.sessions(), (1, 0));
}

#[tokio::test]
async fn logout_attempts_both_deletes_when_both_fail() {
    let w = world();
    let owner = w.owner_token().await;
    let staff = w.staff_token().await;
    {
        let mut state = w.repo.state();
        state.fail_owner_delete = true;
        state.fail_employee_delete = true;
    }

    let answer = w.service.logout(&owner, &staff).await;

    assert!(is_database_failure(&answer), "got {answer:?}");
    let state = w.repo.state();
    assert_eq!(
        (state.owner_deletes, state.employee_deletes),
        (1, 1),
        "each delete is attempted once, whatever the other did"
    );
}

#[tokio::test]
async fn logout_still_ends_the_staff_session_when_the_owner_delete_fails() {
    let w = world();
    let owner = w.owner_token().await;
    let staff = w.staff_token().await;
    w.repo.state().fail_owner_delete = true;

    let answer = w.service.logout(&owner, &staff).await;

    assert!(is_database_failure(&answer), "got {answer:?}");
    assert_eq!(
        w.sessions(),
        (1, 0),
        "the owner's row stays, the staff row is gone"
    );
}

#[tokio::test]
async fn logout_still_ends_the_owner_session_when_the_staff_delete_fails() {
    let w = world();
    let owner = w.owner_token().await;
    let staff = w.staff_token().await;
    w.repo.state().fail_employee_delete = true;

    let answer = w.service.logout(&owner, &staff).await;

    assert!(is_database_failure(&answer), "got {answer:?}");
    assert_eq!(w.sessions(), (0, 1));
}

#[tokio::test]
async fn logout_of_tokens_that_match_no_session_is_ok() {
    let w = world();
    let staff = w.staff_token().await;
    // Thirteen hours later the session is over, but its row is still there.
    assert!(matches!(
        w.service
            .employee_session(&staff, false, at(13 * 3600))
            .await,
        Err(AppError::Unauthenticated)
    ));

    for (owner, staff) in [
        ("", ""),
        (UNKNOWN_TOKEN, UNKNOWN_TOKEN),
        ("not a token", "NOT-A-TOKEN"),
        ("", staff.as_str()),
        ("", staff.as_str()),
    ] {
        let answer = w.service.logout(owner, staff).await;
        assert!(answer.is_ok(), "logout({owner:?}, {staff:?}) → {answer:?}");
    }
    assert_eq!(w.sessions(), (0, 0));
}

#[tokio::test]
async fn logout_with_the_owner_token_alone_touches_no_staff_session() {
    let w = world();
    let owner = w.owner_token().await;
    let _staff = w.staff_token().await;

    let answer = w.service.logout(&owner, "").await;

    assert!(answer.is_ok(), "got {answer:?}");
    assert_eq!(w.sessions(), (0, 1));
    assert_eq!(w.repo.state().employee_deletes, 0);
}

// ---------------------------------------------------------------- status

#[tokio::test]
async fn status_reports_a_failing_staff_session_read() {
    let w = world();
    let staff = w.staff_token().await;
    w.repo.state().fail_employee_session_read = true;

    let answer = w.service.status("", &staff, at(60)).await;

    assert!(
        is_database_failure(&answer),
        "a database failure is not \"nobody is signed in\", got {answer:?}"
    );
}

#[tokio::test]
async fn status_reports_a_failing_owner_session_read() {
    let w = world();
    let owner = w.owner_token().await;
    w.repo.state().fail_owner_session_read = true;

    let answer = w.service.status(&owner, "", at(60)).await;

    assert!(is_database_failure(&answer), "got {answer:?}");
}

#[tokio::test]
async fn employee_session_reports_a_failing_read() {
    let w = world();
    let staff = w.staff_token().await;
    w.repo.state().fail_employee_session_read = true;

    let answer = w.service.employee_session(&staff, false, at(60)).await;

    assert!(is_database_failure(&answer), "got {answer:?}");
}

#[tokio::test]
async fn status_of_a_valid_owner_token_and_no_staff_token_is_the_owner() {
    let w = world();
    let owner = w.owner_token().await;
    // The staff side is not asked at all, so its failure cannot matter.
    w.repo.state().fail_employee_session_read = true;

    let answer = w.service.status(&owner, "", at(60)).await;

    match answer {
        Ok(Some(SignedIn::Owner(who))) => assert_eq!(who.username, "boss"),
        other => panic!("expected the owner, got {other:?}"),
    }
    assert_eq!(w.repo.state().employee_session_reads, 0);
}

#[tokio::test]
async fn status_of_a_staff_token_that_is_no_usable_session_is_none() {
    let w = world();
    let staff = w.staff_token().await;

    // unknown, malformed
    for token in [UNKNOWN_TOKEN, "not a token"] {
        let answer = w.service.status("", token, at(60)).await;
        assert!(
            matches!(answer, Ok(None)),
            "status(\"\", {token:?}) → {answer:?}"
        );
    }
    // expired: at exactly twelve hours it is over
    let answer = w.service.status("", &staff, at(12 * 3600)).await;
    assert!(matches!(answer, Ok(None)), "expired → {answer:?}");
    // a first password still to change
    w.repo.state().employee.as_mut().unwrap().0.must_change = true;
    let answer = w.service.status("", &staff, at(60)).await;
    assert!(matches!(answer, Ok(None)), "must change → {answer:?}");
    w.repo.state().employee.as_mut().unwrap().0.must_change = false;
    // the password changed since (another version)
    w.repo.state().employee.as_mut().unwrap().0.version = 2;
    let answer = w.service.status("", &staff, at(60)).await;
    assert!(matches!(answer, Ok(None)), "version → {answer:?}");
    w.repo.state().employee.as_mut().unwrap().0.version = 1;
    // disabled
    w.repo.state().employee.as_mut().unwrap().0.active = false;
    let answer = w.service.status("", &staff, at(60)).await;
    assert!(matches!(answer, Ok(None)), "disabled → {answer:?}");
    // and back: the same token is the employee again
    w.repo.state().employee.as_mut().unwrap().0.active = true;
    let answer = w.service.status("", &staff, at(60)).await;
    assert!(
        matches!(answer, Ok(Some(SignedIn::Employee(_)))),
        "usable → {answer:?}"
    );
}

// ------------------------------------------------- the spare verification

#[tokio::test]
async fn the_spare_hash_has_the_cost_of_the_services_own_hasher() {
    for (memory, passes, lanes) in [(8, 1, 1), (16, 2, 1), (32, 3, 2)] {
        let hasher = PasswordHasher::with_cost(memory, passes, lanes);
        let own = hasher.hash("a password of this test").expect("hash");
        let service = AccountService::new(Arc::new(FakeRepository::default()), hasher);

        assert_eq!(
            cost_of(service.spare_hash()),
            cost_of(&own),
            "an unknown name must cost what a real account costs"
        );
        assert_eq!(
            cost_of(service.spare_hash()),
            format!("m={memory},t={passes},p={lanes}")
        );
    }
}

#[tokio::test]
async fn the_spare_hash_of_the_real_hasher_has_the_real_cost() {
    let service = AccountService::new(Arc::new(FakeRepository::default()), PasswordHasher::new());

    assert_eq!(cost_of(service.spare_hash()), "m=19456,t=2,p=1");
}

#[tokio::test]
async fn the_spare_hash_is_made_once_and_shared_by_clones() {
    let service = AccountService::new(Arc::new(FakeRepository::default()), cheap_hasher());

    let first = service.spare_hash().to_owned();
    cost_of(&first);
    assert_eq!(
        service.spare_hash(),
        first,
        "a second call makes no new salt"
    );
    assert_eq!(service.clone().spare_hash(), first, "a clone shares it");
}

#[tokio::test]
async fn every_sign_in_that_cannot_succeed_is_refused_alike() {
    let w = world();

    let unknown = w
        .service
        .login_employee(
            &json!({"username": "nobody", "password": STAFF_PASSWORD}),
            "10.0.0.1",
            at(0),
        )
        .await;
    assert!(
        matches!(&unknown, Err(AppError::Domain(sentence)) if sentence == WRONG),
        "unknown name → {unknown:?}"
    );

    // An active account whose password field cannot be verified: missing, not
    // text, empty, or longer than 128 units. The same sentence at both doors,
    // for the employee and for the owner.
    let too_long = "x".repeat(129);
    for name in ["anna", "boss", "nobody"] {
        for payload in [
            json!({"username": name}),
            json!({"username": name, "password": null}),
            json!({"username": name, "password": 12}),
            json!({"username": name, "password": ""}),
            json!({"username": name, "password": too_long}),
        ] {
            let staff = w.service.login_employee(&payload, "10.0.0.1", at(0)).await;
            assert!(
                matches!(&staff, Err(AppError::Domain(sentence)) if sentence == WRONG),
                "active account at the staff door, {payload} → {staff:?}"
            );
            let shared = w.service.login(&payload, "10.0.0.1", at(0)).await;
            assert!(
                matches!(&shared, Err(AppError::Domain(sentence)) if sentence == WRONG),
                "active account at the shared door, {payload} → {shared:?}"
            );
        }
    }
    assert_eq!(w.sessions(), (0, 0), "a refused sign-in leaves no session");
    // … and the right passwords still open both doors.
    let _ = w.owner_token().await;
    let _ = w.staff_token().await;
    assert_eq!(w.sessions(), (1, 1));
    w.repo.state().owner_sessions.clear();
    w.repo.state().employee_sessions.clear();

    // Disabled, with the right password: the same sentence, no session.
    w.repo.state().employee.as_mut().unwrap().0.active = false;
    for payload in [
        json!({"username": "anna", "password": STAFF_PASSWORD}),
        json!({"username": "anna", "password": "wrong password 01"}),
        json!({"username": "anna", "password": 12}),
        json!({"username": "anna"}),
    ] {
        let disabled = w.service.login_employee(&payload, "10.0.0.1", at(0)).await;
        assert!(
            matches!(&disabled, Err(AppError::Domain(sentence)) if sentence == WRONG),
            "disabled account, {payload} → {disabled:?}"
        );
        let shared = w.service.login(&payload, "10.0.0.1", at(0)).await;
        assert!(
            matches!(&shared, Err(AppError::Domain(sentence)) if sentence == WRONG),
            "disabled account at the shared door, {payload} → {shared:?}"
        );
    }
    assert_eq!(w.sessions(), (0, 0), "a refused sign-in leaves no session");
}

// ------------------------------------------ one verification, whatever came

#[test]
fn verify_at_full_cost_answers_as_verify_does_and_the_spare_grants_nothing() {
    const RIGHT: &str = "the right password";
    const SPARE_SOURCE: &str = "what the spare is a hash of";
    let hasher = cheap_hasher();
    let stored = hasher.hash(RIGHT).expect("hash");
    let spare = hasher.hash(SPARE_SOURCE).expect("hash");
    let too_long = "x".repeat(129);

    assert!(hasher.verify_at_full_cost(Some(RIGHT), Some(&stored), &spare));

    for (password, stored, why) in [
        (
            Some("the wrong password"),
            Some(stored.as_str()),
            "a wrong password",
        ),
        (None, Some(stored.as_str()), "no password"),
        (Some(""), Some(stored.as_str()), "an empty password"),
        (Some(too_long.as_str()), Some(stored.as_str()), "129 units"),
        (Some(RIGHT), None, "no account"),
        (Some(RIGHT), Some("-"), "a stored value that is no hash"),
        (None, None, "nothing at all"),
        // the spare is only ever something to spend time on: knowing what it
        // is a hash of opens nothing
        (
            Some(SPARE_SOURCE),
            None,
            "the spare's own source, no account",
        ),
        (
            Some(SPARE_SOURCE),
            Some("-"),
            "the spare's own source, no usable hash",
        ),
        (
            Some(SPARE_SOURCE),
            Some(stored.as_str()),
            "the spare's own source, an account",
        ),
    ] {
        assert!(
            !hasher.verify_at_full_cost(password, stored, &spare),
            "{why} must not verify"
        );
    }
    // a spare that is itself unusable changes no answer
    assert!(hasher.verify_at_full_cost(Some(RIGHT), Some(&stored), ""));
    assert!(!hasher.verify_at_full_cost(Some(RIGHT), None, ""));
}

#[test]
fn a_stored_hash_of_another_argon2_variant_never_verifies() {
    use argon2::password_hash::{PasswordHasher as _, SaltString};
    use argon2::{Algorithm, Argon2, Params, Version};

    const RIGHT: &str = "the right password";
    let hasher = cheap_hasher();
    let spare = hasher.hash("what the spare is a hash of").expect("hash");
    let salt = SaltString::encode_b64(b"saltsaltsaltsalt").expect("a salt");
    for algorithm in [Algorithm::Argon2i, Algorithm::Argon2d] {
        let params = Params::new(8, 1, 1, None).expect("the smallest cost");
        let stored = Argon2::new(algorithm, Version::V0x13, params)
            .hash_password(RIGHT.as_bytes(), &salt)
            .expect("hash")
            .to_string();

        assert!(!hasher.verify(RIGHT, &stored), "{algorithm:?}");
        assert!(
            !hasher.verify_at_full_cost(Some(RIGHT), Some(&stored), &spare),
            "{algorithm:?}"
        );
    }
}
