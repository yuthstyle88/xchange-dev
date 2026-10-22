//! BT-3 (design `docs/design/blueprint/kiosk-shared-phone.md` §7.3 T-D4, T-D7; §7.6; §7.8 rows
//! T6, T9, T10, T14, T16, T23): `PgBranchTabletStore` and `BranchTabletService` on a real
//! PostgreSQL, through the pairing core. Also the migration of §7.6 (BT-0), read from the
//! database.
//!
//! Written before the implementation by the BE lane's skeleton (BT-0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
//! Each test works in a schema of its own with `api/migrations` applied.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use pos_infra::transactions::Transactor;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::tablet_sessions::TabletSessions;
use xchange::infrastructure::branch_tablets::pg_tablet_store::PgBranchTabletStore;
use xchange::infrastructure::branch_tablets::tablet_service::BranchTabletService;
use xchange::infrastructure::branch_tablets::{TabletMe, TabletSeen, TabletState};
use xchange::infrastructure::device_agents::agent_admin::AgentAdmin;
use xchange::infrastructure::device_agents::agent_repository::AgentRepository;
use xchange::infrastructure::device_agents::agent_service::{
    AgentService, PairOutcome as AgentPair, PairParams,
};
use xchange::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::devices::printer_models::PrinterModels;
use xchange::infrastructure::paired_devices::pairing::{PairOutcome, PairingCode};

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ADDRESS: &str = "203.0.113.20";
const MEMBER: &str = "b7000000-0000-4000-8000-000000000001";
const OTHER_MEMBER: &str = "b7000000-0000-4000-8000-000000000002";
const CUSTOMER: &str = "cust-bt-1";
/// A drawer agent's code as `device_agent_http_test.rs` stores it.
const DRAWER_CODE: &str = "7K3QD-9XW2M";
const DRAWER_CODE_CANONICAL: &str = "7K3QD9XW2M";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Now, cut to whole milliseconds (the database keeps microseconds).
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).expect("an instant")
}

fn seen(user_agent: &str) -> TabletSeen<'_> {
    TabletSeen { user_agent }
}

fn close(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_milliseconds().abs() <= 1_000
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
    service: BranchTabletService,
}

impl Room {
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
        let schema = xc_testdb::unique_schema_name("btstore");
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
        let service = BranchTabletService::new(Self::branch_tx_of(&pool));
        Self {
            pool,
            schema,
            service,
        }
    }

    fn branch_tx_of(pool: &PgPool) -> BranchTx {
        BranchTx::new(Transactor::new(pool.clone()))
    }

    fn branch_tx(&self) -> BranchTx {
        Self::branch_tx_of(&self.pool)
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

    async fn sql(&self, statement: &str) {
        sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    async fn count(&self, statement: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(statement.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"))
    }

    async fn text(&self, statement: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(statement.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"))
    }

    /// A tablet of `branch` added and paired through the service: (code as issued, token).
    async fn paired_tablet(&self, branch: &str, name: &str) -> (PairingCode, String) {
        let t = now();
        let code = self
            .service
            .pairing()
            .issue(&BranchId::new(branch), name, "admin-1", t)
            .await
            .unwrap_or_else(|e| panic!("add a tablet to {branch}: {e:?}"));
        match self
            .service
            .pairing()
            .pair(&code.code, &seen("Tablet/1.0"), ADDRESS, t)
            .await
            .expect("pair")
        {
            PairOutcome::Success { token, .. } => (code, token),
            other => panic!("pair the tablet of {branch}: {other:?}"),
        }
    }

    /// The members: two users of the organisation, `MEMBER` behind customer `CUSTOMER`.
    async fn members(&self) {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($1::uuid, $3::uuid, 'Member', 'ACTIVE', 'test'), \
             ($2::uuid, $3::uuid, 'Other', 'ACTIVE', 'test')",
        )
        .bind(MEMBER)
        .bind(OTHER_MEMBER)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("insert the members");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, preferred_language, \
                 status, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $1, 'Member', 'th', 'ACTIVE', 'LIVE', $3::uuid, now())",
        )
        .bind(CUSTOMER)
        .bind(ORG)
        .bind(MEMBER)
        .execute(&self.pool)
        .await
        .expect("insert the member's customer");
    }

    /// A 12-hour session of `user` (as `otp/verify` makes one): its `user_sessions.hash`.
    async fn session(&self, user: &str) -> String {
        let mut conn = self.pool.acquire().await.expect("a connection");
        let token = xc_testdb::sessions::seed_session(
            &mut conn,
            user,
            None,
            Utc::now() + Duration::hours(12),
        )
        .await
        .expect("seed a session");
        sha256_hex(&token)
    }

    async fn session_exists(&self, hash: &str) -> bool {
        self.count(&format!(
            "SELECT count(*) FROM user_sessions WHERE hash = '{hash}'"
        ))
        .await
            == 1
    }

    async fn session_expires(&self, hash: &str) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT expires FROM user_sessions WHERE hash = $1")
            .bind(hash)
            .fetch_one(&self.pool)
            .await
            .expect("read the session")
    }

    /// `(customer_session_hash, customer_user_id)` of a tablet.
    async fn bound(&self, tablet: &str) -> (Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT customer_session_hash, customer_user_id::text FROM branch_tablets WHERE id = $1::uuid",
        )
        .bind(tablet)
        .fetch_one(&self.pool)
        .await
        .expect("read the tablet")
    }
}

// ------------------------------------------------------------ the migration (BT-0)

/// §7.6: one new table with exactly these columns, its three UNIQUE hashes and its branch index,
/// and no one-per-branch index (T-D4); `device_agents`' index is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn the_migration_adds_branch_tablets_and_touches_nothing_else() {
    let room = Room::start().await;
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'branch_tablets' ORDER BY ordinal_position",
    )
    .fetch_all(&room.pool)
    .await
    .expect("read the columns");
    let want = [
        ("id", "uuid", "NO"),
        ("branch_id", "text", "NO"),
        ("name", "text", "NO"),
        ("created_at", "timestamp with time zone", "NO"),
        ("created_by", "text", "NO"),
        ("pair_code_hash", "text", "YES"),
        ("pair_code_expires_at", "timestamp with time zone", "YES"),
        ("token_hash", "text", "YES"),
        ("paired_at", "timestamp with time zone", "YES"),
        ("revoked_at", "timestamp with time zone", "YES"),
        ("revoked_by", "text", "YES"),
        ("last_seen_at", "timestamp with time zone", "YES"),
        ("user_agent", "text", "YES"),
        ("customer_session_hash", "text", "YES"),
        ("customer_user_id", "uuid", "YES"),
    ];
    let want: Vec<(String, String, String)> = want
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect();
    assert_eq!(columns, want);

    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT indexname::text, indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND tablename = 'branch_tablets' ORDER BY indexname",
    )
    .fetch_all(&room.pool)
    .await
    .expect("read the indexes");
    let names: Vec<&str> = indexes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "branch_tablets_branch_idx",
            "branch_tablets_customer_session_hash_key",
            "branch_tablets_pair_code_hash_key",
            "branch_tablets_pkey",
            "branch_tablets_token_hash_key",
        ]
    );
    assert!(
        indexes[0].1.ends_with("(branch_id, created_at, id)"),
        "{}",
        indexes[0].1
    );
    let agent_index = room
        .text(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() \
             AND indexname = 'device_agents_one_active_per_branch_idx'",
        )
        .await
        .expect("the drawer's index is still there");
    assert!(
        agent_index.ends_with("(branch_id) WHERE (revoked_at IS NULL)"),
        "{agent_index}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T6

/// T6: a revoked tablet's token answers `me` unpaired (clearing the cookie); the row keeps its
/// hashes; its bound customer session is deleted and its `customer_*` columns cleared.
#[tokio::test(flavor = "multi_thread")]
async fn t6_revoke_voids_the_token_keeps_the_hashes_and_ends_the_bound_session() {
    let room = Room::start().await;
    room.members().await;
    let (code, token) = room.paired_tablet(MAIN, "front tablet").await;
    let session = room.session(MEMBER).await;
    assert!(
        room.service
            .bind_on_sign_in(&token, &session, MEMBER, now())
            .await
            .expect("bind")
    );

    room.service
        .pairing()
        .revoke(&code.device_id, "admin-1", now())
        .await
        .expect("revoke");

    assert_eq!(
        room.service
            .me(Some(&token), &seen("Tablet/1.0"), ADDRESS, now())
            .await
            .expect("me"),
        TabletMe::Unpaired { clear_cookie: true }
    );
    assert_eq!(
        room.text(&format!(
            "SELECT token_hash FROM branch_tablets WHERE id = '{}'",
            code.device_id
        ))
        .await,
        Some(sha256_hex(&token)),
        "the row keeps its token hash"
    );
    assert!(
        room.text(&format!(
            "SELECT revoked_by FROM branch_tablets WHERE id = '{}'",
            code.device_id
        ))
        .await
        .is_some_and(|by| by == "admin-1")
    );
    assert!(
        !room.session_exists(&session).await,
        "the bound session is deleted"
    );
    assert_eq!(room.bound(&code.device_id).await, (None, None));
    room.finish().await;
}

// ------------------------------------------------------------ me and list

/// `me`: no token ⇒ unpaired without clearing; an unknown one ⇒ unpaired, clearing; a live one ⇒
/// the tablet, with `last_seen_at` and `user_agent` set. `list` names each state.
#[tokio::test(flavor = "multi_thread")]
async fn me_answers_the_live_tablet_and_touches_it_and_list_names_each_state() {
    let room = Room::start().await;
    let s = seen("Mozilla/5.0 (iPad)");
    assert_eq!(
        room.service.me(None, &s, ADDRESS, now()).await.expect("me"),
        TabletMe::Unpaired {
            clear_cookie: false
        }
    );
    assert_eq!(
        room.service
            .me(Some("xcbt_00ff"), &s, ADDRESS, now())
            .await
            .expect("me"),
        TabletMe::Unpaired { clear_cookie: true }
    );

    let (code, token) = room.paired_tablet(B9, "door tablet").await;
    let t = now() + Duration::seconds(5);
    assert_eq!(
        room.service
            .me(Some(&token), &s, ADDRESS, t)
            .await
            .expect("me"),
        TabletMe::Paired {
            tablet_id: code.device_id.clone(),
            branch_code: B9.to_owned(),
            name: "door tablet".to_owned(),
        }
    );
    let (last_seen, agent): (Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT last_seen_at, user_agent FROM branch_tablets WHERE id = $1::uuid")
            .bind(&code.device_id)
            .fetch_one(&room.pool)
            .await
            .expect("read the tablet");
    assert_eq!(last_seen, Some(t));
    assert_eq!(agent.as_deref(), Some("Mozilla/5.0 (iPad)"));

    let pending = room
        .service
        .pairing()
        .issue(&BranchId::new(MAIN), "pending", "admin-1", now())
        .await
        .expect("add");
    let (revoked, _) = room.paired_tablet(MAIN, "gone").await;
    room.service
        .pairing()
        .revoke(&revoked.device_id, "admin-1", now())
        .await
        .expect("revoke");

    let listed = room.service.list(now()).await.expect("list");
    let states: Vec<(String, String, TabletState)> = listed
        .iter()
        .map(|l| (l.branch_id.clone(), l.tablet_id.clone(), l.state))
        .collect();
    let mut want = vec![
        (B9.to_owned(), code.device_id.clone(), TabletState::Paired),
        (
            MAIN.to_owned(),
            pending.device_id.clone(),
            TabletState::Pending,
        ),
        (
            MAIN.to_owned(),
            revoked.device_id.clone(),
            TabletState::Revoked,
        ),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(states, want, "by branch, then oldest first");
    room.finish().await;
}

// ------------------------------------------------------------ T9, T10

/// T9: a branch with an active drawer agent takes three tablets; the drawer's readers answer as
/// before (one active agent, nothing else listed as an agent), and B9 still takes its own drawer.
#[tokio::test(flavor = "multi_thread")]
async fn t9_tablets_beside_a_drawer_agent_change_nothing_for_the_drawer() {
    let room = Room::start().await;
    room.sql(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, pair_code_hash, \
         pair_code_expires_at) VALUES ('a0000000-0000-4000-8000-000000000001'::uuid, 'MAIN', 'drawer', \
         now(), 'owner', 'h', now() + interval '10 minutes')",
    )
    .await;
    for i in 0..3 {
        room.service
            .pairing()
            .issue(
                &BranchId::new(MAIN),
                &format!("tablet {i}"),
                "admin-1",
                now(),
            )
            .await
            .unwrap_or_else(|e| panic!("tablet {i}: {e:?}"));
    }
    assert_eq!(
        room.count("SELECT count(*) FROM branch_tablets WHERE branch_id = 'MAIN'")
            .await,
        3
    );
    assert_eq!(room.count("SELECT count(*) FROM device_agents").await, 1);

    let agents = PgAgentRepository::new(room.pool.clone());
    let pool = room.pool.clone();
    let has = room
        .branch_tx()
        .run(&BranchId::new(B9), move |tx| {
            let agents = PgAgentRepository::new(pool.clone());
            Box::pin(async move {
                let main = agents.has_active_agent_in(tx, &BranchId::new(MAIN)).await?;
                let b9 = agents.has_active_agent_in(tx, &BranchId::new(B9)).await?;
                Ok((main, b9))
            })
        })
        .await
        .expect("has_active_agent_in");
    assert_eq!(has, (true, false), "the tablets are not drawer agents");
    assert_eq!(agents.list().await.expect("list").len(), 1);

    let admin = AgentAdmin::new(room.branch_tx());
    assert_eq!(admin.list(now()).await.expect("agent list").len(), 1);
    admin
        .add(B9, "drawer nine", "owner", now())
        .await
        .expect("B9 still takes a drawer agent");
    room.finish().await;
}

/// T10: a drawer agent's code does not pair a tablet, and a tablet's code does not pair a drawer
/// agent: each kind's code is looked up only in its own table.
#[tokio::test(flavor = "multi_thread")]
async fn t10_a_drawer_code_and_a_tablet_code_do_not_cross() {
    let room = Room::start().await;
    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, pair_code_hash, \
         pair_code_expires_at) VALUES ('a0000000-0000-4000-8000-000000000001'::uuid, 'MAIN', 'drawer', \
         now(), 'owner', $1, now() + interval '10 minutes')",
    )
    .bind(sha256_hex(DRAWER_CODE_CANONICAL))
    .execute(&room.pool)
    .await
    .expect("a pending drawer agent");
    assert_eq!(
        room.service
            .pairing()
            .pair(DRAWER_CODE, &seen("Tablet/1.0"), ADDRESS, now())
            .await
            .expect("pair"),
        PairOutcome::InvalidCode
    );

    let tablet = room
        .service
        .pairing()
        .issue(&BranchId::new(B9), "tablet", "admin-1", now())
        .await
        .expect("add");
    let agent_service = AgentService::new(
        Arc::new(PgAgentRepository::new(room.pool.clone())),
        Arc::new(PgDeviceRepository::new(room.pool.clone())),
        PgAttemptCounter::new(room.pool.clone()),
        room.branch_tx(),
        PrinterModels::built_in(),
    );
    let params = PairParams {
        code: &tablet.code,
        host: "h",
        os: "o",
        version: "1",
        build: "b",
    };
    assert_eq!(
        agent_service
            .pair(&params, "203.0.113.21", now())
            .await
            .expect("agent pair"),
        AgentPair::InvalidCode
    );
    assert_eq!(
        room.count("SELECT count(*) FROM branch_tablets WHERE paired_at IS NULL")
            .await,
        1,
        "the tablet is still pending"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T14, T23

/// T14: the sign-in binds the new session to the tablet and deletes the one bound before. A
/// revoked tablet's token, an unknown one and an unpaired tablet bind nothing.
#[tokio::test(flavor = "multi_thread")]
async fn t14_the_sign_in_binds_the_session_and_replaces_the_one_before() {
    let room = Room::start().await;
    room.members().await;
    let (code, token) = room.paired_tablet(MAIN, "front tablet").await;

    let first = room.session(MEMBER).await;
    assert!(
        room.service
            .bind_on_sign_in(&token, &first, MEMBER, now())
            .await
            .expect("bind")
    );
    assert_eq!(
        room.bound(&code.device_id).await,
        (Some(first.clone()), Some(MEMBER.to_owned()))
    );
    assert!(room.service.is_bound_session(&first).await.expect("bound?"));

    let second = room.session(OTHER_MEMBER).await;
    assert!(
        room.service
            .bind_on_sign_in(&token, &second, OTHER_MEMBER, now())
            .await
            .expect("bind")
    );
    assert_eq!(
        room.bound(&code.device_id).await,
        (Some(second.clone()), Some(OTHER_MEMBER.to_owned()))
    );
    assert!(
        !room.session_exists(&first).await,
        "the session bound before is deleted"
    );
    assert!(room.session_exists(&second).await);
    assert!(!room.service.is_bound_session(&first).await.expect("bound?"));

    let loose = room.session(MEMBER).await;
    assert!(
        !room
            .service
            .bind_on_sign_in("xcbt_unknown", &loose, MEMBER, now())
            .await
            .expect("bind")
    );
    let pending = room
        .service
        .pairing()
        .issue(&BranchId::new(MAIN), "pending", "admin-1", now())
        .await
        .expect("add");
    assert_eq!(room.bound(&pending.device_id).await, (None, None));
    room.service
        .pairing()
        .revoke(&code.device_id, "admin-1", now())
        .await
        .expect("revoke");
    assert!(
        !room
            .service
            .bind_on_sign_in(&token, &loose, MEMBER, now())
            .await
            .expect("bind")
    );
    assert!(
        room.session_exists(&loose).await,
        "nothing is bound or deleted"
    );
    assert!(!room.service.is_bound_session(&loose).await.expect("bound?"));
    room.finish().await;
}

/// T23 (Q-T1 A): binding caps the session at sign-in + 30 minutes; a session not bound keeps
/// its 12 hours.
#[tokio::test(flavor = "multi_thread")]
async fn t23_binding_caps_the_session_at_thirty_minutes() {
    let room = Room::start().await;
    room.members().await;
    let (_, token) = room.paired_tablet(MAIN, "front tablet").await;
    let bound = room.session(MEMBER).await;
    let phone = room.session(MEMBER).await;
    let t = now();
    assert!(
        room.service
            .bind_on_sign_in(&token, &bound, MEMBER, t)
            .await
            .expect("bind")
    );
    let expires = room.session_expires(&bound).await;
    assert!(
        close(expires, t + Duration::minutes(30)),
        "{expires} is sign-in + 30 min ({t})"
    );
    let untouched = room.session_expires(&phone).await;
    assert!(
        untouched > t + Duration::hours(11),
        "the phone session keeps 12 h: {untouched}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T16 and the end

/// T16 and `end_for_member_in`: the scan's end at MAIN deletes only the member's session bound to
/// a MAIN tablet and clears that tablet; the member's phone session and a B9 tablet's session
/// stay. The port answers as the inherent method.
#[tokio::test(flavor = "multi_thread")]
async fn t16_the_end_deletes_only_the_session_bound_to_a_tablet_of_that_branch() {
    let room = Room::start().await;
    room.members().await;
    let (main_tablet, main_token) = room.paired_tablet(MAIN, "main tablet").await;
    let (b9_tablet, b9_token) = room.paired_tablet(B9, "b9 tablet").await;
    let on_main = room.session(MEMBER).await;
    let on_b9 = room.session(MEMBER).await;
    let phone = room.session(MEMBER).await;
    assert!(
        room.service
            .bind_on_sign_in(&main_token, &on_main, MEMBER, now())
            .await
            .expect("bind")
    );
    assert!(
        room.service
            .bind_on_sign_in(&b9_token, &on_b9, MEMBER, now())
            .await
            .expect("bind")
    );

    let store = PgBranchTabletStore::new(room.pool.clone());
    let ended = room
        .branch_tx()
        .run(&BranchId::new(MAIN), move |tx| {
            let store = store.clone();
            Box::pin(async move {
                let main = BranchId::new(MAIN);
                let nobody = store.end_for_member_in(tx, &main, "cust-unknown").await?;
                let member = store.end_for_member_in(tx, &main, CUSTOMER).await?;
                Ok((nobody, member))
            })
        })
        .await
        .expect("end_for_member_in");
    assert_eq!(ended, (0, 1));
    assert!(
        !room.session_exists(&on_main).await,
        "the MAIN tablet's session is deleted"
    );
    assert_eq!(room.bound(&main_tablet.device_id).await, (None, None));
    assert!(room.session_exists(&phone).await, "the phone session stays");
    assert!(
        room.session_exists(&on_b9).await,
        "the B9 tablet's session stays"
    );
    assert_eq!(
        room.bound(&b9_tablet.device_id).await,
        (Some(on_b9.clone()), Some(MEMBER.to_owned()))
    );

    let port: Arc<dyn TabletSessions> = Arc::new(PgBranchTabletStore::new(room.pool.clone()));
    let again = room
        .branch_tx()
        .run(&BranchId::new(B9), move |tx| {
            let port = port.clone();
            Box::pin(async move {
                let b9 = BranchId::new(B9);
                port.end_for_member_in(tx, &b9, CUSTOMER).await
            })
        })
        .await
        .expect("the port");
    assert_eq!(again, 1);
    assert!(!room.session_exists(&on_b9).await);
    assert!(room.session_exists(&phone).await);
    room.finish().await;
}
