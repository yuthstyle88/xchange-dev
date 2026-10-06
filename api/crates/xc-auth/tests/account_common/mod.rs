//! Shared by the account tests: a PostgreSQL schema per test, the v2 fixture
//! and the replay that holds `AccountService` to it.
//! Provided with ticket T4b — do not edit.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::application::account::service::{AccountService, EmployeesAnswer};
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::owner::Owner;
use xc_auth::domain::account::entities::session::{Login, SignedIn};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;

pub type Service = AccountService<PgAccountRepository>;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(60);

/// v2's sentences (`server.mjs:7293`, `7315`, `7322`, `7338`, `7341`, `7333`, `7350`).
pub const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
pub const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
pub const PASSWORD_LENGTH: &str = "รหัสผ่านต้องมี 12–128 ตัวอักษร";
pub const EMPLOYEE_DATA: &str = "ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง";
pub const EMPLOYEE_NOT_FOUND: &str = "ไม่พบพนักงาน";
pub const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
pub const MUST_CHANGE: &str = "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน";
/// What v2 throws when there is no usable staff session; in v3 it is `AppError::Unauthenticated`.
pub const AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// Await `fut`, failing the test when it takes longer than [`MUST_FINISH`].
pub async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(MUST_FINISH, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {MUST_FINISH:?}: {what}"),
    }
}

/// A fixed instant, `seconds` after 2026-10-07 03:00:00 UTC — the start of a 15-minute window.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

pub fn at_millis(millis: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(millis)
        .single()
        .expect("a valid instant")
}

/// The sentence a refusal carries, or [`AUTH_REQUIRED`] for `Unauthenticated`.
/// Anything else is not a refusal and fails the test.
pub fn refusal(error: &AppError) -> String {
    match error {
        AppError::Domain(sentence) => sentence.clone(),
        AppError::Unauthenticated => AUTH_REQUIRED.to_owned(),
        other => panic!(
            "expected a refusal (AppError::Domain with v2's sentence, or AppError::Unauthenticated), got {other:?}"
        ),
    }
}

/// The hasher the service tests use: Argon2id at its smallest cost, so that a
/// story of hundreds of sign-ins takes seconds.
pub fn cheap_hasher() -> PasswordHasher {
    PasswordHasher::with_cost(8, 1, 1)
}

pub struct TestDb {
    pub pool: PgPool,
    pub schema: String,
}

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    ///
    /// `TEST_DATABASE_URL` must name an existing database whose name ends in
    /// `_test`. When it is absent the test FAILS; nothing here skips.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The account tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("t4b");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        Self { pool, schema }
    }

    pub fn service(&self) -> Service {
        self.service_with(cheap_hasher())
    }

    pub fn service_with(&self, hasher: PasswordHasher) -> Service {
        AccountService::new(
            Arc::new(PgAccountRepository::new(self.pool.clone())),
            hasher,
        )
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// The audit lines, in no particular order: (id, actor, subject, action, at).
    pub async fn audit(&self) -> Vec<(String, String, String, String, DateTime<Utc>)> {
        sqlx::query_as("SELECT id::text, actor, subject, action, at FROM employee_audit")
            .fetch_all(&self.pool)
            .await
            .expect("read employee_audit")
    }

    pub async fn drop_schema(self) {
        let Self { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// A valid `create` payload.
pub fn new_employee(username: &str, role: &str, branch: &str, password: &str) -> Value {
    json!({ "username": username, "name": format!("คุณ {username}"), "role": role, "branchId": branch, "password": password })
}

/// Creates an employee and gives back the row.
pub async fn create_employee(service: &Service, payload: &Value, now: DateTime<Utc>) -> Employee {
    let answer = service
        .employees("create", payload, "owner", now)
        .await
        .unwrap_or_else(|e| panic!("create {payload}: {e:?}"));
    assert!(
        matches!(answer, EmployeesAnswer::Saved),
        "create answers Saved"
    );
    let wanted = payload["username"]
        .as_str()
        .expect("username")
        .trim()
        .to_lowercase();
    list(service, now)
        .await
        .into_iter()
        .find(|e| e.username == wanted)
        .unwrap_or_else(|| panic!("the employee {wanted} is listed after create"))
}

pub async fn list(service: &Service, now: DateTime<Utc>) -> Vec<Employee> {
    match service
        .employees("list", &json!({}), "owner", now)
        .await
        .expect("list employees")
    {
        EmployeesAnswer::List(people) => people,
        EmployeesAnswer::Saved => panic!("list answers List"),
    }
}

/// Creates an employee who has already replaced the first password with `password`.
pub async fn working_employee(
    service: &Service,
    username: &str,
    password: &str,
    now: DateTime<Utc>,
) -> Employee {
    let first = "the-first-password";
    create_employee(
        service,
        &new_employee(username, "FRONT", "MAIN", first),
        now,
    )
    .await;
    let login = service
        .login_employee(
            &json!({ "username": username, "password": first }),
            &unique("setup-address"),
            now,
        )
        .await
        .expect("first sign-in");
    service
        .change_password(&login.token, &json!({ "password": password }), now)
        .await
        .expect("change the first password");
    list(service, now)
        .await
        .into_iter()
        .find(|e| e.username == username)
        .expect("listed")
}

// ── the v2 fixture ───────────────────────────────────────────────────────

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_account_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is JSON")
}

pub fn story(fixture: &Value, name: &str) -> Value {
    fixture["stories"]
        .as_array()
        .expect("stories")
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("the fixture has no story '{name}'"))
        .clone()
}

fn iso(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Replays one story of the fixture against a service on a fresh schema.
pub struct Replay {
    pub db: TestDb,
    pub service: Service,
    tokens: HashMap<String, String>,
    employee_ids: Vec<String>,
    employees: Value,
    seen_audit: HashSet<String>,
}

impl Replay {
    pub async fn run(story: &Value) {
        let db = TestDb::start().await;
        let service = db.service();
        let steps = story["steps"].as_array().expect("steps");
        let begin = at_millis(steps[0]["at"].as_i64().expect("at"));
        let owner = &story["owner"];
        let made = service
            .create_owner(
                owner["username"].as_str().unwrap(),
                owner["password"].as_str().unwrap(),
                begin,
            )
            .await
            .expect("the story's owner is created");
        assert_eq!(made.username, owner["username"]);

        let mut replay = Self {
            db,
            service,
            tokens: HashMap::new(),
            employee_ids: Vec::new(),
            employees: json!([]),
            seen_audit: HashSet::new(),
        };
        for (index, step) in steps.iter().enumerate() {
            let label = format!(
                "story '{}', step {index} ({}): {}",
                story["name"].as_str().unwrap(),
                step["op"].as_str().unwrap(),
                step["note"].as_str().unwrap()
            );
            within(&label, replay.step(step, &label)).await;
        }
        replay.db.drop_schema().await;
    }

    fn token(&self, value: &Value) -> String {
        match value.as_str() {
            None => String::new(),
            Some(s) if s.starts_with("$t") => self
                .tokens
                .get(s)
                .unwrap_or_else(|| panic!("the fixture uses token {s} before v2 made it"))
                .clone(),
            Some(s) => s.to_owned(),
        }
    }

    fn real_id(&self, symbol: &str) -> Option<String> {
        let n: usize = symbol.strip_prefix("$e")?.parse().ok()?;
        self.employee_ids.get(n.checked_sub(1)?).cloned()
    }

    fn symbol_of(&self, id: &str) -> String {
        match self.employee_ids.iter().position(|x| x == id) {
            Some(i) => format!("$e{}", i + 1),
            None => id.to_owned(),
        }
    }

    /// The payload with `$eN` replaced by the id v3 gave that employee.
    fn resolve(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.real_id(s).unwrap_or_else(|| s.clone())),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.resolve(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.resolve(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// v2's `safe(employee)` (`server.mjs:7304`).
    fn safe(&self, e: &Employee) -> Value {
        json!({
            "id": self.symbol_of(&e.id), "username": e.username, "name": e.name, "role": e.role,
            "branchId": e.branch_id, "active": e.active, "mustChange": e.must_change,
        })
    }

    /// v2's `publicUser` (`server.mjs:7839`). The owner's `id` is left out: v2 answers the
    /// constant `OWNER_ID`, which is the HTTP layer's to add (ticket T4c).
    fn public_user(&self, who: &SignedIn) -> Value {
        match who {
            SignedIn::Owner(o) => {
                json!({ "username": o.username, "name": o.name, "role": "OWNER", "branchId": null })
            }
            SignedIn::Employee(e) => json!({
                "id": self.symbol_of(&e.id), "username": e.username, "name": e.name,
                "role": e.role, "branchId": e.branch_id,
            }),
        }
    }

    fn expected_user(user: &Value) -> Value {
        let mut user = user.clone();
        if user["role"] == "OWNER" {
            user.as_object_mut().unwrap().remove("id");
        }
        user
    }

    async fn learn_new_employees(&mut self, now: DateTime<Utc>) -> Vec<Employee> {
        let people = list(&self.service, now).await;
        let mut fresh: Vec<&Employee> = people
            .iter()
            .filter(|e| !self.employee_ids.contains(&e.id))
            .collect();
        assert!(fresh.len() <= 1, "one step creates at most one employee");
        if let Some(e) = fresh.pop() {
            self.employee_ids.push(e.id.clone());
        }
        people
    }

    fn remember_token(&mut self, symbol: &Value, login: &Login, label: &str) {
        let symbol = symbol
            .as_str()
            .unwrap_or_else(|| panic!("{label}: the fixture names the token"));
        assert!(
            login.token.len() == 64
                && login
                    .token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{label}: a session token is 64 lower-case hex characters, got {:?}",
            login.token
        );
        assert!(
            !self.tokens.values().any(|t| t == &login.token),
            "{label}: every sign-in gets a token of its own"
        );
        self.tokens.insert(symbol.to_owned(), login.token.clone());
    }

    async fn step(&mut self, step: &Value, label: &str) {
        let now = at_millis(step["at"].as_i64().expect("at"));
        let op = step["op"].as_str().unwrap();
        let accepted = step.get("ok");
        let refused = step.get("refused").and_then(Value::as_str).map(|sentence| {
            // v2 answers a taken username with the database's own error text. v3 answers the
            // sentence for employee data that is not acceptable (ticket T4b, "Decided").
            if sentence.starts_with("duplicate key value") {
                EMPLOYEE_DATA
            } else {
                sentence
            }
        });
        let ip = step["ip"].as_str().unwrap_or("unknown");

        // What v3 answered, in the fixture's shape.
        let answer: Result<Value, AppError> = match op {
            "login" => match self.service.login(&step["payload"], ip, now).await {
                Ok(login) => {
                    let set = &step["setCookies"];
                    let mut body = json!({ "user": self.public_user(&login.who) });
                    match &login.who {
                        SignedIn::Owner(_) => {
                            self.remember_token(&set["owner"]["token"], &login, label)
                        }
                        SignedIn::Employee(e) => {
                            body["mustChange"] = json!(e.must_change);
                            self.remember_token(&set["staff"]["token"], &login, label)
                        }
                    }
                    Ok(body)
                }
                Err(e) => Err(e),
            },
            "staffLogin" => match self.service.login_employee(&step["payload"], ip, now).await {
                Ok(login) => {
                    self.remember_token(&step["ok"]["token"], &login, label);
                    match &login.who {
                        SignedIn::Employee(e) => {
                            Ok(json!({ "token": step["ok"]["token"], "employee": self.safe(e) }))
                        }
                        SignedIn::Owner(_) => panic!("{label}: the staff door signed an owner in"),
                    }
                }
                Err(e) => Err(e),
            },
            "status" => {
                let owner = self.token(&step["cookies"]["owner"]);
                let staff = self.token(&step["cookies"]["staff"]);
                self.service.status(&owner, &staff, now).await.map(|who| {
                    json!({
                        "setupRequired": false,
                        "user": who.map(|w| self.public_user(&w)).unwrap_or(Value::Null),
                    })
                })
            }
            "logout" => {
                let owner = self.token(&step["cookies"]["owner"]);
                let staff = self.token(&step["cookies"]["staff"]);
                self.service
                    .logout(&owner, &staff)
                    .await
                    .map(|()| json!({ "saved": true }))
            }
            "staffLogout" => self
                .service
                .logout("", &self.token(&step["token"]))
                .await
                .map(|()| json!({ "saved": true })),
            "session" => self
                .service
                .employee_session(
                    &self.token(&step["token"]),
                    step["allowChange"].as_bool().unwrap(),
                    now,
                )
                .await
                .map(|e| self.safe(&e)),
            "password" => self
                .service
                .change_password(&self.token(&step["token"]), &step["payload"], now)
                .await
                .map(|()| json!({ "saved": true, "loginAgain": true })),
            "employees" => {
                let payload = self.resolve(&step["payload"]);
                match self
                    .service
                    .employees(
                        step["action"].as_str().unwrap(),
                        &payload,
                        step["actor"].as_str().unwrap(),
                        now,
                    )
                    .await
                {
                    Ok(EmployeesAnswer::Saved) => {
                        self.learn_new_employees(now).await;
                        Ok(json!({ "saved": true }))
                    }
                    Ok(EmployeesAnswer::List(mut people)) => {
                        people.sort_by(|a, b| a.username.cmp(&b.username));
                        Ok(
                            json!({ "employees": people.iter().map(|e| self.safe(e)).collect::<Vec<_>>() }),
                        )
                    }
                    Err(e) => Err(e),
                }
            }
            "ownerActive" => {
                let changed = sqlx::query("UPDATE owners SET active = $1")
                    .bind(step["active"].as_bool().unwrap())
                    .execute(&self.db.pool)
                    .await
                    .expect("set owners.active");
                assert_eq!(changed.rows_affected(), 1, "{label}: there is one owner");
                return;
            }
            other => {
                panic!("{label}: the fixture has an operation this replay does not know: {other}")
            }
        };

        match (answer, accepted, refused) {
            (Ok(got), Some(want), None) => {
                let mut want = want.clone();
                if let Some(user) = want.get("user").filter(|u| !u.is_null()).cloned() {
                    want["user"] = Self::expected_user(&user);
                }
                assert_eq!(got, want, "{label}: v2 accepted with this answer");
            }
            (Err(error), None, Some(sentence)) => {
                assert_eq!(
                    refusal(&error),
                    sentence,
                    "{label}: v2 refused with this sentence"
                );
            }
            (Ok(got), None, Some(sentence)) => {
                panic!("{label}: v2 refused with «{sentence}», v3 accepted with {got}")
            }
            (Err(error), Some(want), None) => {
                panic!("{label}: v2 accepted with {want}, v3 refused with {error:?}")
            }
            _ => panic!("{label}: a step is either accepted or refused"),
        }

        // What is stored: every employee row, and the audit lines this step added.
        if let Some(employees) = step.get("employees") {
            self.employees = employees.clone();
        }
        let mut people = self.learn_new_employees(now).await;
        people.sort_by(|a, b| a.username.cmp(&b.username));
        let stored: Vec<Value> = people
            .iter()
            .map(|e| {
                let mut row = self.safe(e);
                row["version"] = json!(e.version);
                row["createdAt"] = json!(iso(e.created_at));
                row
            })
            .collect();
        assert_eq!(
            Value::Array(stored),
            self.employees,
            "{label}: the employee rows after the step (a refused step changes nothing)"
        );

        let mut added: Vec<Value> = Vec::new();
        for (id, actor, subject, action, at) in self.db.audit().await {
            if self.seen_audit.insert(id) {
                added.push(json!({
                    "actor": self.symbol_of(&actor), "subject": self.symbol_of(&subject),
                    "action": action, "at": iso(at),
                }));
            }
        }
        let want = step.get("audit").cloned().unwrap_or_else(|| json!([]));
        assert_eq!(
            Value::Array(added),
            want,
            "{label}: the audit lines the step added (a refused step adds none)"
        );
    }
}

/// How many of each outcome a set of concurrent calls produced.
pub fn tally<T>(results: Vec<Result<T, AppError>>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for result in results {
        let key = match result {
            Ok(_) => "ok".to_owned(),
            Err(e) => refusal(&e),
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

pub fn owner_of(who: &SignedIn) -> &Owner {
    match who {
        SignedIn::Owner(o) => o,
        SignedIn::Employee(e) => panic!("expected the owner, got the employee {}", e.username),
    }
}

pub fn employee_of(who: &SignedIn) -> &Employee {
    match who {
        SignedIn::Employee(e) => e,
        SignedIn::Owner(o) => panic!("expected an employee, got the owner {}", o.username),
    }
}
