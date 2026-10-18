//! RBAC F11a (#3205) tests for `SessionUser.must_change_password` on login, session read and refresh,
//! and `SessionService::change_password`.

mod account_common;

use std::sync::{Arc, Mutex};

use account_common::{TestDb, unique};
use chrono::{Duration, Utc};
use futures::future::BoxFuture;
use xc_auth::application::session::{SessionError, SessionService, sha256_hex};
use xc_auth::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityLoginResponse, IdentityOtpRequestResponse,
    IdentityOtpVerifyResponse, IdentityRegisterRequest, IdentityRegisterResponse,
};
use xc_auth::domain::token::{TokenError, TokenVerifier, VerifiedToken};

const LOGIN_ACCESS_TOKEN: &str = "login-access-token-01";
const LOGIN_REFRESH_TOKEN: &str = "login-refresh-token-01";
const REFRESHED_ACCESS_TOKEN: &str = "refreshed-access-token-99";
const REFRESHED_REFRESH_TOKEN: &str = "refreshed-refresh-token-99";
const CLIENT_SECRET: &str = "client-secret-for-f11a-tests-0001";

#[derive(Default)]
struct FakeState {
    login_must_change: Option<bool>,
    change_password_result: Option<Result<(), IdentityError>>,
    refresh_calls: Vec<(String, Option<String>)>,
    change_password_calls: Vec<(String, String, String, Option<String>)>,
}

#[derive(Default)]
struct FakeIdentity {
    state: Mutex<FakeState>,
}

impl FakeIdentity {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn set_login_must_change(&self, value: Option<bool>) {
        self.state.lock().unwrap().login_must_change = value;
    }

    fn set_change_password_result(&self, result: Result<(), IdentityError>) {
        self.state.lock().unwrap().change_password_result = Some(result);
    }

    fn refresh_calls(&self) -> Vec<(String, Option<String>)> {
        self.state.lock().unwrap().refresh_calls.clone()
    }

    fn change_password_calls(&self) -> Vec<(String, String, String, Option<String>)> {
        self.state.lock().unwrap().change_password_calls.clone()
    }
}

impl IdentityClient for FakeIdentity {
    fn login<'a>(
        &'a self,
        _identifier: &'a str,
        _password: &'a str,
        _client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        let must_change_password = self.state.lock().unwrap().login_must_change;
        Box::pin(async move {
            Ok(IdentityLoginResponse {
                identity_id: "id-1".to_string(),
                access_token: LOGIN_ACCESS_TOKEN.to_string(),
                token_type: Some("Bearer".to_string()),
                expires_in: Some(900),
                refresh_token: Some(LOGIN_REFRESH_TOKEN.to_string()),
                phone: None,
                email_verified: None,
                phone_verified: None,
                must_change_password,
            })
        })
    }

    fn refresh<'a>(
        &'a self,
        refresh_token: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        self.state
            .lock()
            .unwrap()
            .refresh_calls
            .push((refresh_token.to_string(), client_ip.map(ToOwned::to_owned)));
        Box::pin(async move {
            Ok(IdentityLoginResponse {
                identity_id: "id-1".to_string(),
                access_token: REFRESHED_ACCESS_TOKEN.to_string(),
                token_type: Some("Bearer".to_string()),
                expires_in: Some(900),
                refresh_token: Some(REFRESHED_REFRESH_TOKEN.to_string()),
                phone: None,
                email_verified: None,
                phone_verified: None,
                must_change_password: None,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _refresh_token: &'a str,
        _client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        Box::pin(async { Ok(()) })
    }

    fn otp_request<'a>(
        &'a self,
        _destination: &'a str,
        _channel: &'a str,
        _client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpRequestResponse, IdentityError>> {
        unimplemented!()
    }

    fn otp_verify<'a>(
        &'a self,
        _challenge_id: &'a str,
        _code: &'a str,
        _client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpVerifyResponse, IdentityError>> {
        unimplemented!()
    }

    fn register<'a>(
        &'a self,
        _request: &'a IdentityRegisterRequest<'a>,
    ) -> BoxFuture<'a, Result<IdentityRegisterResponse, IdentityError>> {
        unimplemented!()
    }

    fn change_password<'a>(
        &'a self,
        access_token: &'a str,
        current_password: &'a str,
        new_password: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        let mut st = self.state.lock().unwrap();
        st.change_password_calls.push((
            access_token.to_string(),
            current_password.to_string(),
            new_password.to_string(),
            client_ip.map(ToOwned::to_owned),
        ));
        let res = st.change_password_result.clone().unwrap_or(Ok(()));
        Box::pin(async move { res })
    }
}

struct FakeVerifier {
    subject: String,
}

impl TokenVerifier for FakeVerifier {
    fn verify<'a>(&'a self, _jwt: &'a str) -> BoxFuture<'a, Result<VerifiedToken, TokenError>> {
        let subject = self.subject.clone();
        Box::pin(async move {
            Ok(VerifiedToken {
                subject,
                expires_at: 4_102_444_800,
                phone: None,
            })
        })
    }
}

async fn seed_active_user(db: &TestDb) -> (String, String) {
    let subject = unique("sub");
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         SELECT id, $1, 'Somchai', 'ACTIVE', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .bind(&subject)
    .fetch_one(&db.pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT $1::uuid, 'TELLER', id, 'MAIN' FROM organizations WHERE code = 'XC'",
    )
    .bind(&user_id)
    .execute(&db.pool)
    .await
    .expect("insert user_role");

    (user_id, subject)
}

fn make_service(db: &TestDb, identity: Arc<FakeIdentity>, subject: &str) -> SessionService {
    SessionService::new(
        db.pool.clone(),
        Some(identity),
        Some(Arc::new(FakeVerifier {
            subject: subject.to_string(),
        })),
        Some(CLIENT_SECRET.to_string()),
    )
}

async fn stored_must_change_password(db: &TestDb, session_token: &str) -> Option<bool> {
    sqlx::query_scalar("SELECT must_change_password FROM user_sessions WHERE hash = $1")
        .bind(sha256_hex(session_token))
        .fetch_optional(&db.pool)
        .await
        .expect("query user_sessions")
}

/// Row 1: Identity's login says `must_change_password: true` -> the login answer's
/// `user.must_change_password` is true, and so is the stored row.
#[tokio::test]
async fn login_when_identity_says_must_change_password_true_sets_answer_and_stored_row() {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    identity.set_login_must_change(Some(true));
    let svc = make_service(&db, identity, &subject);
    let now = Utc::now();

    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login succeeds");

    assert!(
        tail.user.must_change_password,
        "login answer must have must_change_password = true"
    );
    assert_eq!(
        stored_must_change_password(&db, &tail.session_token).await,
        Some(true),
        "user_sessions.must_change_password must be true"
    );

    db.drop_schema().await;
}

/// Row 2: Identity's login says `false` or omits `must_change_password` -> `false` (while `true` -> `true`).
#[tokio::test]
async fn login_when_identity_says_false_or_omits_it_answers_false() {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    let svc = make_service(&db, identity.clone(), &subject);
    let now = Utc::now();

    identity.set_login_must_change(Some(false));
    let explicit_false = svc
        .login("somchai", "pw", Some("203.0.113.1"), now)
        .await
        .expect("login with false");
    assert!(!explicit_false.user.must_change_password);
    assert_eq!(
        stored_must_change_password(&db, &explicit_false.session_token).await,
        Some(false)
    );

    identity.set_login_must_change(None);
    let omitted = svc
        .login("somchai", "pw", Some("203.0.113.1"), now)
        .await
        .expect("login with omitted flag");
    assert!(!omitted.user.must_change_password);
    assert_eq!(
        stored_must_change_password(&db, &omitted.session_token).await,
        Some(false)
    );

    identity.set_login_must_change(Some(true));
    let flagged = svc
        .login("somchai", "pw", Some("203.0.113.1"), now)
        .await
        .expect("login with true");
    assert!(flagged.user.must_change_password);

    db.drop_schema().await;
}

/// Row 3: The session read (`session_check`) and `refresh` of a flagged session answer `must_change_password: true`.
#[tokio::test]
async fn session_read_and_refresh_of_flagged_session_answer_must_change_password_true() {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    identity.set_login_must_change(Some(true));
    let svc = make_service(&db, identity.clone(), &subject);
    let now = Utc::now();

    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login");

    let checked = svc
        .session_check(&tail.session_token, now)
        .await
        .expect("session_check")
        .expect("active session");
    assert!(
        checked.must_change_password,
        "session_check must answer must_change_password = true for a flagged session"
    );

    let refreshed = svc
        .refresh(&tail.session_token, Some("203.0.113.1"), now)
        .await
        .expect("refresh");
    assert!(
        refreshed.must_change_password,
        "refresh must answer must_change_password = true for a flagged session"
    );

    identity.set_login_must_change(Some(false));
    let normal = svc
        .login("somchai", "normal-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("normal login");
    let normal_checked = svc
        .session_check(&normal.session_token, now)
        .await
        .expect("session_check")
        .expect("active session");
    assert!(!normal_checked.must_change_password);
    let normal_refreshed = svc
        .refresh(&normal.session_token, Some("203.0.113.1"), now)
        .await
        .expect("refresh");
    assert!(!normal_refreshed.must_change_password);

    db.drop_schema().await;
}

/// Row 4: `change_password` with the right current password -> `Ok(())`; Identity's `change_password`
/// got the access token from `refresh`; the session row is gone.
#[tokio::test]
async fn change_password_with_right_current_password_passes_refreshed_access_token_and_deletes_session()
 {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    identity.set_login_must_change(Some(true));
    let svc = make_service(&db, identity.clone(), &subject);
    let now = Utc::now();

    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login");

    let result = svc
        .change_password(
            &tail.session_token,
            "temp-pass-1",
            "NewStrongPass#2026",
            Some("203.0.113.9"),
            now,
        )
        .await;
    assert_eq!(result, Ok(()));

    assert_eq!(
        identity.refresh_calls(),
        vec![(
            LOGIN_REFRESH_TOKEN.to_string(),
            Some("203.0.113.9".to_string())
        )],
        "change_password must decrypt the stored refresh token and call Identity::refresh"
    );
    assert_eq!(
        identity.change_password_calls(),
        vec![(
            REFRESHED_ACCESS_TOKEN.to_string(),
            "temp-pass-1".to_string(),
            "NewStrongPass#2026".to_string(),
            Some("203.0.113.9".to_string()),
        )],
        "Identity::change_password must receive the access token returned by refresh"
    );

    assert_eq!(
        stored_must_change_password(&db, &tail.session_token).await,
        None,
        "the session row must be deleted after a successful password change"
    );
    assert!(
        svc.session_check(&tail.session_token, now)
            .await
            .expect("session_check")
            .is_none(),
        "session_check must return None after session is deleted"
    );

    db.drop_schema().await;
}

/// Row 5: Identity refuses (wrong current password) -> `Err(SessionError::Identity(..))` with Identity's error;
/// the row is still there.
#[tokio::test]
async fn change_password_when_identity_refuses_wrong_current_password_returns_error_and_keeps_session()
 {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    identity.set_login_must_change(Some(true));
    identity.set_change_password_result(Err(IdentityError::InvalidCredentials));
    let svc = make_service(&db, identity, &subject);
    let now = Utc::now();

    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login");

    let err = svc
        .change_password(
            &tail.session_token,
            "wrong-current-pass",
            "NewStrongPass#2026",
            Some("203.0.113.9"),
            now,
        )
        .await;
    assert_eq!(
        err,
        Err(SessionError::Identity(IdentityError::InvalidCredentials))
    );

    assert_eq!(
        stored_must_change_password(&db, &tail.session_token).await,
        Some(true),
        "session row must remain when Identity refuses with InvalidCredentials"
    );

    db.drop_schema().await;
}

/// Row 6: Identity refuses the new password (policy) -> `Err(SessionError::Identity(..))`; the row stays.
#[tokio::test]
async fn change_password_when_identity_refuses_new_password_policy_returns_error_and_keeps_session()
{
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    identity.set_login_must_change(Some(true));
    identity.set_change_password_result(Err(IdentityError::WeakPassword));
    let svc = make_service(&db, identity, &subject);
    let now = Utc::now();

    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login");

    let err = svc
        .change_password(
            &tail.session_token,
            "temp-pass-1",
            "weak",
            Some("203.0.113.9"),
            now,
        )
        .await;
    assert_eq!(
        err,
        Err(SessionError::Identity(IdentityError::WeakPassword))
    );

    assert_eq!(
        stored_must_change_password(&db, &tail.session_token).await,
        Some(true),
        "session row must remain when Identity refuses with WeakPassword"
    );

    db.drop_schema().await;
}

/// Row 7: No live session for the token -> the missing-session refusal; Identity is never called.
#[tokio::test]
async fn change_password_with_no_live_session_returns_sign_in_and_never_calls_identity() {
    let db = TestDb::start().await;
    let (_user_id, subject) = seed_active_user(&db).await;
    let identity = FakeIdentity::new();
    let svc = make_service(&db, identity.clone(), &subject);
    let now = Utc::now();

    // Missing session token
    let missing_err = svc
        .change_password(
            &"ab".repeat(32),
            "temp-pass-1",
            "NewStrongPass#2026",
            Some("203.0.113.9"),
            now,
        )
        .await;
    assert_eq!(
        missing_err,
        Err(SessionError::Identity(IdentityError::SignIn))
    );

    // Expired session row
    let tail = svc
        .login("somchai", "temp-pass-1", Some("203.0.113.1"), now)
        .await
        .expect("login");
    let expired_err = svc
        .change_password(
            &tail.session_token,
            "temp-pass-1",
            "NewStrongPass#2026",
            Some("203.0.113.9"),
            now + Duration::seconds(50_000),
        )
        .await;
    assert_eq!(
        expired_err,
        Err(SessionError::Identity(IdentityError::SignIn))
    );

    assert!(
        identity.refresh_calls().is_empty(),
        "Identity::refresh must not be called when there is no live session"
    );
    assert!(
        identity.change_password_calls().is_empty(),
        "Identity::change_password must not be called when there is no live session"
    );

    db.drop_schema().await;
}
