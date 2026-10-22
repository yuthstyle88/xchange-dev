//! BT-1 (design `docs/design/blueprint/kiosk-shared-phone.md` §7.3 T-D2, §7.5, §7.8 rows T1–T5,
//! T7): the one pairing core, `DeviceSecret` and `DevicePairing<S>`, for **both kinds through one
//! table**: every row runs once for `DeviceKind::DrawerAgent` and once for
//! `DeviceKind::BranchTablet` over the same fake `PairedDeviceStore`, so a rule that holds for one
//! kind and not the other fails.
//!
//! Written before the implementation by the BE lane's skeleton (BT-0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The store is a fake that keeps its rows in memory and follows the statements' contracts in
//! `paired_devices/store.rs`. The branch transaction and the attempt counter are the real ones,
//! so the database is real: a schema of the test's own with `api/migrations` applied. Needs
//! `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::infrastructure::device_agents::agent_token::AgentToken;
use xchange::infrastructure::paired_devices::kind::{DeviceKind, PerBranch};
use xchange::infrastructure::paired_devices::pairing::{
    AuthOutcome, DevicePairing, PairOutcome, PairingCode, PairingError,
};
use xchange::infrastructure::paired_devices::secret::DeviceSecret;
use xchange::infrastructure::paired_devices::store::{
    NewPaired, PairedDevice, PairedDeviceStore, Tx,
};

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const ADDRESS: &str = "203.0.113.10";
const OTHER_ADDRESS: &str = "203.0.113.11";
const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 2026-10-22 03:00:00 UTC (the start of a 15-minute window) plus `seconds`.
fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 3, 0, 0)
        .single()
        .expect("an instant")
        + Duration::seconds(seconds)
}

const MINUTE: i64 = 60;

// ------------------------------------------------------------ the fake store

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeDevice {
    id: String,
    branch_id: String,
    name: String,
    created_by: String,
    code_hash: Option<String>,
    code_expires_at: Option<DateTime<Utc>>,
    token_hash: Option<String>,
    paired_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    revoked_by: Option<String>,
    seen: Option<String>,
}

impl PairedDevice for FakeDevice {
    fn id(&self) -> &str {
        &self.id
    }
    fn branch_id(&self) -> &str {
        &self.branch_id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// The fake store of one kind: `TABLET = false` is the drawer agent, `true` the branch tablet.
#[derive(Default)]
struct Fake<const TABLET: bool> {
    rows: Mutex<Vec<FakeDevice>>,
    /// `(branch, id, actor)` of every `after_revoke_in`.
    after_revoke: Mutex<Vec<(String, String, String)>>,
}

impl<const TABLET: bool> Fake<TABLET> {
    fn rows(&self) -> Vec<FakeDevice> {
        self.rows.lock().unwrap().clone()
    }

    fn row(&self, id: &str) -> FakeDevice {
        self.rows()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("device {id} is stored"))
    }
}

impl<const TABLET: bool> PairedDeviceStore for Fake<TABLET> {
    const KIND: DeviceKind = if TABLET {
        DeviceKind::BranchTablet
    } else {
        DeviceKind::DrawerAgent
    };
    type Device = FakeDevice;
    type Seen<'a> = &'a str;

    async fn insert_pending_in(
        &self,
        _tx: Tx<'_, '_>,
        branch: &BranchId,
        device: &NewPaired<'_>,
    ) -> AppResult<bool> {
        let mut rows = self.rows.lock().unwrap();
        if Self::KIND.per_branch() == PerBranch::One
            && rows
                .iter()
                .any(|d| d.branch_id == branch.as_str() && d.revoked_at.is_none())
        {
            return Ok(false);
        }
        rows.push(FakeDevice {
            id: device.id.to_owned(),
            branch_id: branch.as_str().to_owned(),
            name: device.name.to_owned(),
            created_by: device.created_by.to_owned(),
            code_hash: Some(device.pair_code_hash.to_owned()),
            code_expires_at: Some(device.pair_code_expires_at),
            token_hash: None,
            paired_at: None,
            revoked_at: None,
            revoked_by: None,
            seen: None,
        });
        Ok(true)
    }

    async fn set_code_in(
        &self,
        _tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut rows = self.rows.lock().unwrap();
        let Some(d) = rows
            .iter_mut()
            .find(|d| d.branch_id == branch.as_str() && d.id == id && d.revoked_at.is_none())
        else {
            return Ok(false);
        };
        d.code_hash = Some(pair_code_hash.to_owned());
        d.code_expires_at = Some(expires_at);
        Ok(true)
    }

    async fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &Self::Seen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<FakeDevice>> {
        let mut rows = self.rows.lock().unwrap();
        let Some(d) = rows.iter_mut().find(|d| {
            d.code_hash.as_deref() == Some(pair_code_hash)
                && d.revoked_at.is_none()
                && d.code_expires_at.is_some_and(|e| e > now)
        }) else {
            return Ok(None);
        };
        d.token_hash = Some(token_hash.to_owned());
        d.paired_at = Some(now);
        d.code_hash = None;
        d.code_expires_at = None;
        d.seen = Some((*seen).to_owned());
        Ok(Some(d.clone()))
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> AppResult<Option<FakeDevice>> {
        Ok(self
            .rows()
            .into_iter()
            .find(|d| d.token_hash.as_deref() == Some(token_hash)))
    }

    async fn find(&self, id: &str) -> AppResult<Option<FakeDevice>> {
        Ok(self.rows().into_iter().find(|d| d.id == id))
    }

    async fn revoke_in(
        &self,
        _tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut rows = self.rows.lock().unwrap();
        let Some(d) = rows
            .iter_mut()
            .find(|d| d.branch_id == branch.as_str() && d.id == id && d.revoked_at.is_none())
        else {
            return Ok(false);
        };
        d.revoked_at = Some(now);
        d.revoked_by = Some(actor.to_owned());
        Ok(true)
    }

    async fn after_revoke_in(
        &self,
        _tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.after_revoke.lock().unwrap().push((
            branch.as_str().to_owned(),
            id.to_owned(),
            actor.to_owned(),
        ));
        Ok(())
    }

    async fn list(&self) -> AppResult<Vec<FakeDevice>> {
        Ok(self.rows())
    }
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    /// A schema of the test's own with `api/migrations` applied and branches MAIN and B9.
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = xc_testdb::unique_schema_name("btcore");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        for b in [MAIN, B9] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("branch {b}: {e}"));
        }
        Self { pool, schema }
    }

    fn pairing<const TABLET: bool>(&self) -> (Arc<Fake<TABLET>>, DevicePairing<Fake<TABLET>>) {
        let store = Arc::new(Fake::<TABLET>::default());
        let pairing = DevicePairing::new(
            store.clone(),
            BranchTx::new(Transactor::new(self.pool.clone())),
            PgAttemptCounter::new(self.pool.clone()),
        );
        (store, pairing)
    }

    /// The `auth_attempts` count of `key` in the window starting at `at(0)`, 0 when none.
    async fn attempts(&self, key: &str) -> i32 {
        sqlx::query_scalar::<_, i32>(
            "SELECT count FROM auth_attempts WHERE key = $1 AND window_start = $2",
        )
        .bind(key)
        .bind(at(0))
        .fetch_optional(&self.pool)
        .await
        .expect("read auth_attempts")
        .unwrap_or(0)
    }

    async fn finish(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }
}

/// Runs one row for both kinds, each in a room of its own.
macro_rules! both_kinds {
    ($row:ident) => {{
        let room = Room::start().await;
        $row::<false>(&room).await;
        room.finish().await;
        let room = Room::start().await;
        $row::<true>(&room).await;
        room.finish().await;
    }};
}

fn kind_of<const TABLET: bool>() -> DeviceKind {
    <Fake<TABLET> as PairedDeviceStore>::KIND
}

fn main_branch() -> BranchId {
    BranchId::new(MAIN)
}

fn code_shape(code: &str) -> bool {
    let b = code.as_bytes();
    code.len() == 11
        && b[5] == b'-'
        && code
            .chars()
            .enumerate()
            .all(|(i, c)| i == 5 || CROCKFORD.contains(c))
}

fn is_token_of(kind: DeviceKind, token: &str) -> bool {
    token.strip_prefix(kind.token_prefix()).is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

async fn issued<const TABLET: bool>(
    pairing: &DevicePairing<Fake<TABLET>>,
    branch: &str,
    t: DateTime<Utc>,
) -> PairingCode {
    pairing
        .issue(&BranchId::new(branch), "  tablet one  ", "admin-1", t)
        .await
        .unwrap_or_else(|e| panic!("{:?}: issue at {branch}: {e:?}", kind_of::<TABLET>()))
}

async fn paired<const TABLET: bool>(
    pairing: &DevicePairing<Fake<TABLET>>,
    code: &str,
    t: DateTime<Utc>,
) -> (FakeDevice, String) {
    match pairing.pair(code, &"seen-1", ADDRESS, t).await {
        Ok(PairOutcome::Success { device, token }) => (device, token),
        other => panic!("{:?}: pair {code}: {other:?}", kind_of::<TABLET>()),
    }
}

// ------------------------------------------------------------ the secrets (no database)

/// The kind's constants (BT-0) are the drawer agent's today (`agent_service.rs:93,115,131`) and
/// the tablet's of §7.5.
#[test]
fn each_kind_has_its_prefix_its_attempt_keys_and_its_per_branch_rule() {
    assert_eq!(DeviceKind::DrawerAgent.token_prefix(), "xcda_");
    assert_eq!(DeviceKind::BranchTablet.token_prefix(), "xcbt_");
    assert_eq!(DeviceKind::DrawerAgent.pair_key("1.2.3.4"), "PAIR:1.2.3.4");
    assert_eq!(
        DeviceKind::BranchTablet.pair_key("1.2.3.4"),
        "TABLET_PAIR:1.2.3.4"
    );
    assert_eq!(
        DeviceKind::DrawerAgent.token_key("1.2.3.4"),
        "AGENT:1.2.3.4"
    );
    assert_eq!(
        DeviceKind::BranchTablet.token_key("1.2.3.4"),
        "TABLET:1.2.3.4"
    );
    assert_eq!(DeviceKind::DrawerAgent.per_branch(), PerBranch::One);
    assert_eq!(DeviceKind::BranchTablet.per_branch(), PerBranch::Many);
}

/// `DeviceSecret` is `AgentToken` generalised: the same hash and code hash, and a token with the
/// kind's prefix; the code is 10 Crockford characters shown `XXXXX-XXXXX`.
#[test]
fn the_secrets_have_the_kinds_prefix_and_the_codes_shape_and_hash_as_agent_token_does() {
    for kind in [DeviceKind::DrawerAgent, DeviceKind::BranchTablet] {
        let a = DeviceSecret::new_token(kind);
        let b = DeviceSecret::new_token(kind);
        assert!(is_token_of(kind, &a), "{kind:?}: {a}");
        assert_ne!(a, b, "{kind:?}: two tokens differ");
    }
    for _ in 0..50 {
        let code = DeviceSecret::new_pairing_code();
        assert!(code_shape(&code), "{code}");
        let canonical = code.replace('-', "");
        assert_eq!(
            DeviceSecret::code_hash(&code),
            Some(DeviceSecret::hash(&canonical))
        );
    }
    assert_eq!(
        DeviceSecret::hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(DeviceSecret::hash("xcbt_x"), AgentToken::hash("xcbt_x"));

    let h = DeviceSecret::code_hash("7K3QD-9XW2M").expect("a valid code");
    assert_eq!(h, DeviceSecret::hash("7K3QD9XW2M"));
    assert_eq!(
        DeviceSecret::code_hash(" 7k3qd 9xw2m "),
        Some(h.clone()),
        "case and spaces"
    );
    assert_eq!(
        DeviceSecret::code_hash("7K3QD9XW2M"),
        Some(h.clone()),
        "no dash"
    );
    assert_eq!(
        AgentToken::code_hash("7K3QD-9XW2M"),
        Some(h),
        "the drawer's hash is the same"
    );
    for bad in [
        "",
        "7K3QD-9XW2",
        "7K3QD-9XW2MM",
        "7K3QD-9XW2I",
        "7K3QD-9XW2L",
        "7K3QD-9XW2O",
        "7K3QD-9XW2U",
        "7K3QD_9XW2M",
    ] {
        assert_eq!(DeviceSecret::code_hash(bad), None, "{bad:?}");
    }
}

// ------------------------------------------------------------ T1

async fn t1<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (store, pairing) = room.pairing::<TABLET>();
    let issued = issued(&pairing, MAIN, at(0)).await;
    assert!(code_shape(&issued.code), "{kind:?}: {}", issued.code);
    assert_eq!(
        issued.code_expires_at,
        at(10 * MINUTE),
        "{kind:?}: valid 10 min"
    );

    let row = store.row(&issued.device_id);
    assert!(
        uuid::Uuid::parse_str(&row.id).is_ok_and(|u| u.to_string() == row.id),
        "{kind:?}: a lower-case UUID id: {}",
        row.id
    );
    assert_eq!(row.branch_id, MAIN);
    assert_eq!(row.name, "tablet one", "{kind:?}: the name is trimmed");
    assert_eq!(row.created_by, "admin-1");
    assert_eq!(
        row.code_hash,
        DeviceSecret::code_hash(&issued.code),
        "{kind:?}: only the code's hash is stored"
    );
    assert_eq!(row.code_expires_at, Some(at(10 * MINUTE)));
    assert_eq!(
        (row.token_hash, row.paired_at, row.revoked_at),
        (None, None, None),
        "{kind:?}: PENDING"
    );
}

/// T1: `add` stores a PENDING device with a code `XXXXX-XXXXX` valid 10 minutes, shown once;
/// only its hash is stored.
#[tokio::test(flavor = "multi_thread")]
async fn t1_issue_stores_a_pending_device_with_a_ten_minute_code_kept_only_as_its_hash() {
    both_kinds!(t1);
}

async fn issue_refusals<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (store, pairing) = room.pairing::<TABLET>();
    for name in ["", "   ", &"ก".repeat(61)] {
        assert!(
            matches!(
                pairing.issue(&main_branch(), name, "admin-1", at(0)).await,
                Err(PairingError::NameRequired)
            ),
            "{kind:?}: name {name:?}"
        );
    }
    assert!(
        pairing
            .issue(&main_branch(), &"ก".repeat(60), "admin-1", at(0))
            .await
            .is_ok(),
        "{kind:?}: 60 UTF-16 units is allowed"
    );
    assert!(
        matches!(
            pairing
                .issue(&BranchId::new("NOPE"), "x", "admin-1", at(0))
                .await,
            Err(PairingError::App(_))
        ),
        "{kind:?}: an unknown branch"
    );
    assert_eq!(
        store.rows().len(),
        1,
        "{kind:?}: only the good one is stored"
    );
}

/// The name is 1 to 60 UTF-16 units after trimming, and the branch must exist
/// (`agent_admin.rs:147-155`), for both kinds.
#[tokio::test(flavor = "multi_thread")]
async fn issue_refuses_a_bad_name_and_an_unknown_branch() {
    both_kinds!(issue_refusals);
}

// ------------------------------------------------------------ T2, T3, T4

async fn t2<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (store, pairing) = room.pairing::<TABLET>();
    let issued = issued(&pairing, MAIN, at(0)).await;
    let lower = issued.code.to_lowercase();
    let (device, token) = paired(&pairing, &lower, at(9 * MINUTE)).await;
    assert!(is_token_of(kind, &token), "{kind:?}: {token}");
    assert_eq!(device.id, issued.device_id);
    let row = store.row(&issued.device_id);
    assert_eq!(
        row.token_hash,
        Some(DeviceSecret::hash(&token)),
        "{kind:?}: only the token's hash"
    );
    assert_eq!(row.paired_at, Some(at(9 * MINUTE)));
    assert_eq!(
        (row.code_hash, row.code_expires_at),
        (None, None),
        "{kind:?}: the code is used up"
    );
    assert_eq!(
        row.seen.as_deref(),
        Some("seen-1"),
        "{kind:?}: what the device said is stored"
    );
}

/// T2: a code 9 minutes old pairs, in any case; the token has the kind's prefix and only its
/// hash is stored; the code is used up.
#[tokio::test(flavor = "multi_thread")]
async fn t2_a_live_code_pairs_once_and_only_the_tokens_hash_is_stored() {
    both_kinds!(t2);
}

async fn t3_t4<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (store, pairing) = room.pairing::<TABLET>();
    let old = issued(&pairing, MAIN, at(0)).await;
    assert_eq!(
        pairing
            .pair(&old.code, &"s", ADDRESS, at(11 * MINUTE))
            .await
            .expect("pair"),
        PairOutcome::InvalidCode,
        "{kind:?}: T3, 11 minutes old"
    );
    assert_eq!(
        store.row(&old.device_id).paired_at,
        None,
        "{kind:?}: nothing paired"
    );

    let used = issued(&pairing, B9, at(MINUTE)).await;
    paired(&pairing, &used.code, at(2 * MINUTE)).await;
    let again = pairing
        .pair(&used.code, &"s", ADDRESS, at(3 * MINUTE))
        .await
        .expect("pair");
    let unknown = pairing
        .pair("00000-00000", &"s", ADDRESS, at(3 * MINUTE))
        .await
        .expect("pair");
    let malformed = pairing
        .pair("not a code", &"s", ADDRESS, at(3 * MINUTE))
        .await
        .expect("pair");
    assert_eq!(
        (again, unknown, malformed),
        (
            PairOutcome::InvalidCode,
            PairOutcome::InvalidCode,
            PairOutcome::InvalidCode
        ),
        "{kind:?}: T4, a used code answers as an expired, an unknown and a malformed one"
    );
}

/// T3, T4: an expired code, a used one, an unknown one and a malformed one are one answer.
#[tokio::test(flavor = "multi_thread")]
async fn t3_t4_expired_used_unknown_and_malformed_codes_are_one_answer() {
    both_kinds!(t3_t4);
}

// ------------------------------------------------------------ T5

async fn t5<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (_, pairing) = room.pairing::<TABLET>();
    let good = issued(&pairing, MAIN, at(0)).await;
    for i in 0..10 {
        assert_eq!(
            pairing
                .pair("00000-00000", &"s", ADDRESS, at(i))
                .await
                .expect("pair"),
            PairOutcome::InvalidCode,
            "{kind:?}: attempt {}",
            i + 1
        );
    }
    assert_eq!(
        pairing
            .pair(&good.code, &"s", ADDRESS, at(20))
            .await
            .expect("pair"),
        PairOutcome::RateLimited,
        "{kind:?}: the 11th attempt in 15 minutes, even with a live code"
    );
    let key = DeviceSecret::hash(&kind.pair_key(ADDRESS));
    assert_eq!(
        room.attempts(&key).await,
        11,
        "{kind:?}: every attempt counted under its key"
    );

    // A new window: the same address pairs again (a renewed code, as the first one has expired).
    let fresh = pairing
        .renew(&good.device_id, "admin-1", at(15 * MINUTE))
        .await
        .expect("renew");
    paired(&pairing, &fresh.code, at(15 * MINUTE + 1)).await;

    let other = issued(&pairing, B9, at(0)).await;
    match pairing
        .pair(&other.code, &"s", OTHER_ADDRESS, at(30))
        .await
        .expect("pair")
    {
        PairOutcome::Success { .. } => {}
        o => panic!("{kind:?}: another address is not limited: {o:?}"),
    }
}

/// T5: 10 pair attempts per address and 15-minute window; the 11th is `RateLimited`; a new
/// window or another address pairs.
#[tokio::test(flavor = "multi_thread")]
async fn t5_the_eleventh_pair_attempt_from_one_address_in_fifteen_minutes_is_rate_limited() {
    both_kinds!(t5);
}

/// The kinds count pair attempts under different keys: ten failed drawer pairs from an address
/// do not limit a tablet pair from it.
#[tokio::test(flavor = "multi_thread")]
async fn the_kinds_count_pair_attempts_apart() {
    let room = Room::start().await;
    let (_, drawer) = room.pairing::<false>();
    let (_, tablet) = room.pairing::<true>();
    for i in 0..10 {
        drawer
            .pair("00000-00000", &"s", ADDRESS, at(i))
            .await
            .expect("pair");
    }
    let code = issued(&tablet, MAIN, at(0)).await;
    match tablet
        .pair(&code.code, &"s", ADDRESS, at(20))
        .await
        .expect("pair")
    {
        PairOutcome::Success { .. } => {}
        o => panic!("the tablet's pair is not limited by the drawer's attempts: {o:?}"),
    }
    assert_eq!(
        room.attempts(&DeviceSecret::hash(
            &DeviceKind::DrawerAgent.pair_key(ADDRESS)
        ))
        .await,
        10
    );
    assert_eq!(
        room.attempts(&DeviceSecret::hash(
            &DeviceKind::BranchTablet.pair_key(ADDRESS)
        ))
        .await,
        1
    );
    room.finish().await;
}

// ------------------------------------------------------------ T7 and the token

async fn t7<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (_, pairing) = room.pairing::<TABLET>();
    let first = issued(&pairing, MAIN, at(0)).await;
    let (_, old) = paired(&pairing, &first.code, at(MINUTE)).await;

    let renewed = pairing
        .renew(&first.device_id, "admin-1", at(2 * MINUTE))
        .await
        .expect("renew");
    assert_eq!(renewed.device_id, first.device_id);
    assert!(code_shape(&renewed.code));
    assert_eq!(
        renewed.code_expires_at,
        at(12 * MINUTE),
        "{kind:?}: valid 10 min from the renew"
    );
    assert!(
        matches!(
            pairing
                .authenticate(Some(&old), ADDRESS, at(3 * MINUTE))
                .await
                .expect("auth"),
            AuthOutcome::Ok(_)
        ),
        "{kind:?}: the old token works until the new code is used"
    );

    let (_, new) = paired(&pairing, &renewed.code, at(4 * MINUTE)).await;
    assert_ne!(new, old);
    match pairing
        .authenticate(Some(&new), ADDRESS, at(5 * MINUTE))
        .await
        .expect("auth")
    {
        AuthOutcome::Ok(d) => assert_eq!(d.id, first.device_id),
        o => panic!("{kind:?}: the new token works: {o:?}"),
    }
    assert_eq!(
        pairing
            .authenticate(Some(&old), ADDRESS, at(5 * MINUTE))
            .await
            .expect("auth"),
        AuthOutcome::InvalidToken,
        "{kind:?}: the old token is void"
    );
}

/// T7: a new code re-pairs the device: the new token works and the old one is void.
#[tokio::test(flavor = "multi_thread")]
async fn t7_a_new_code_re_pairs_and_voids_the_old_token() {
    both_kinds!(t7);
}

async fn tokens<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let other = if TABLET {
        DeviceKind::DrawerAgent
    } else {
        DeviceKind::BranchTablet
    };
    let (store, pairing) = room.pairing::<TABLET>();
    let code = issued(&pairing, MAIN, at(0)).await;
    let (_, token) = paired(&pairing, &code.code, at(MINUTE)).await;

    // A token of the other kind whose hash is stored is still refused: the prefix is checked first.
    let foreign = DeviceSecret::new_token(other);
    store.rows.lock().unwrap()[0].token_hash = Some(DeviceSecret::hash(&foreign));
    assert_eq!(
        pairing
            .authenticate(Some(&foreign), ADDRESS, at(2 * MINUTE))
            .await
            .expect("auth"),
        AuthOutcome::InvalidToken,
        "{kind:?}: the other kind's prefix"
    );
    store.rows.lock().unwrap()[0].token_hash = Some(DeviceSecret::hash(&token));
    match pairing
        .authenticate(Some(&format!("  {token} ")), ADDRESS, at(2 * MINUTE))
        .await
        .expect("auth")
    {
        AuthOutcome::Ok(_) => {}
        o => panic!("{kind:?}: the token is trimmed: {o:?}"),
    }

    let unknown = DeviceSecret::new_token(kind);
    for t in [None, Some(""), Some(unknown.as_str())] {
        assert_eq!(
            pairing
                .authenticate(t, ADDRESS, at(3 * MINUTE))
                .await
                .expect("auth"),
            AuthOutcome::InvalidToken,
            "{kind:?}: {t:?}"
        );
    }
    pairing
        .revoke(&code.device_id, "admin-1", at(4 * MINUTE))
        .await
        .expect("revoke");
    assert_eq!(
        pairing
            .authenticate(Some(&token), ADDRESS, at(5 * MINUTE))
            .await
            .expect("auth"),
        AuthOutcome::InvalidToken,
        "{kind:?}: a revoked device's token"
    );
    // 5 failures so far; 25 more make 30, the 31st is limited.
    for i in 0..25 {
        assert_eq!(
            pairing
                .authenticate(None, ADDRESS, at(6 * MINUTE + i))
                .await
                .expect("auth"),
            AuthOutcome::InvalidToken,
            "{kind:?}: failure {}",
            i + 6
        );
    }
    assert_eq!(
        pairing
            .authenticate(None, ADDRESS, at(7 * MINUTE))
            .await
            .expect("auth"),
        AuthOutcome::RateLimited,
        "{kind:?}: the 31st failure in 15 minutes"
    );
    assert_eq!(
        room.attempts(&DeviceSecret::hash(&kind.token_key(ADDRESS)))
            .await,
        31
    );
}

/// A missing token, the other kind's, an unknown one and a revoked device's each count one
/// failure under the kind's token key; the 31st in 15 minutes is `RateLimited`.
#[tokio::test(flavor = "multi_thread")]
async fn failed_token_checks_are_counted_per_kind_and_limited_at_thirty() {
    both_kinds!(tokens);
}

// ------------------------------------------------------------ per branch, revoke, branch_of

/// The drawer stays one per branch (a revoked one frees the branch); a branch may have any
/// number of tablets (T-D4, T9's core half).
#[tokio::test(flavor = "multi_thread")]
async fn the_drawer_is_one_per_branch_and_tablets_are_many() {
    let room = Room::start().await;
    let (_, drawer) = room.pairing::<false>();
    let first = issued(&drawer, MAIN, at(0)).await;
    assert!(
        matches!(
            drawer
                .issue(&main_branch(), "second", "admin-1", at(1))
                .await,
            Err(PairingError::BranchHasOne)
        ),
        "a second drawer agent at MAIN"
    );
    issued(&drawer, B9, at(2)).await;
    drawer
        .revoke(&first.device_id, "admin-1", at(3))
        .await
        .expect("revoke");
    issued(&drawer, MAIN, at(4)).await;

    let (tablets, tablet) = room.pairing::<true>();
    for i in 0..3 {
        issued(&tablet, MAIN, at(10 + i)).await;
    }
    assert_eq!(tablets.rows().len(), 3, "three tablets at MAIN");
    room.finish().await;
}

async fn revoke_and_branch_of<const TABLET: bool>(room: &Room) {
    let kind = kind_of::<TABLET>();
    let (store, pairing) = room.pairing::<TABLET>();
    let code = issued(&pairing, B9, at(0)).await;
    assert_eq!(
        pairing.branch_of(&code.device_id).await.expect("branch_of"),
        Some(BranchId::new(B9))
    );
    assert_eq!(
        pairing
            .branch_of(&code.device_id.to_uppercase())
            .await
            .expect("branch_of"),
        Some(BranchId::new(B9)),
        "{kind:?}: any UUID spelling"
    );
    for bad in ["", "not-a-uuid", "a0000000-0000-4000-8000-000000000999"] {
        assert_eq!(
            pairing.branch_of(bad).await.expect("branch_of"),
            None,
            "{kind:?}: {bad:?}"
        );
        assert!(
            matches!(
                pairing.renew(bad, "a", at(1)).await,
                Err(PairingError::NoSuchDevice)
            ),
            "{kind:?}: renew {bad:?}"
        );
        assert!(
            matches!(
                pairing.revoke(bad, "a", at(1)).await,
                Err(PairingError::NoSuchDevice)
            ),
            "{kind:?}: revoke {bad:?}"
        );
    }

    pairing
        .revoke(&code.device_id, "admin-2", at(2))
        .await
        .expect("revoke");
    let row = store.row(&code.device_id);
    assert_eq!(
        (row.revoked_at, row.revoked_by.as_deref()),
        (Some(at(2)), Some("admin-2"))
    );
    assert_eq!(
        store.after_revoke.lock().unwrap().clone(),
        vec![(B9.to_owned(), code.device_id.clone(), "admin-2".to_owned())],
        "{kind:?}: after_revoke_in once, after the revoke"
    );
    assert!(
        matches!(
            pairing.revoke(&code.device_id, "a", at(3)).await,
            Err(PairingError::NoSuchDevice)
        ),
        "{kind:?}: twice"
    );
    assert!(
        matches!(
            pairing.renew(&code.device_id, "a", at(3)).await,
            Err(PairingError::NoSuchDevice)
        ),
        "{kind:?}: renew a revoked one"
    );
    assert_eq!(
        store.after_revoke.lock().unwrap().len(),
        1,
        "{kind:?}: no second after_revoke_in"
    );
    assert_eq!(
        pairing.branch_of(&code.device_id).await.expect("branch_of"),
        Some(BranchId::new(B9)),
        "{kind:?}: a revoked device still names its branch"
    );
}

/// `revoke` revokes once and then runs the kind's `after_revoke_in`; an unknown, malformed or
/// revoked id is `NoSuchDevice` for `renew` and `revoke`; `branch_of` answers the branch.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_runs_after_revoke_once_and_branch_of_names_the_branch() {
    both_kinds!(revoke_and_branch_of);
}
