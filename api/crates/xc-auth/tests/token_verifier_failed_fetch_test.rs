//! #689: a failed JWKS refetch must start the #660 refetch window too. Before this, `resolve_key`
//! returned on the fetch error before recording the attempt, so while the Identity was down every
//! token with an unknown `kid` sent another GET to the JWKS endpoint.
//!
//! The lane controller's ruling on #689 (2026-10-09T11:2xZ):
//! - a failed fetch starts the window, like a successful one;
//! - inside the window that follows a **failed** attempt, an unknown `kid` answers `KeysUnavailable`
//!   (the key set is unknown, so this is an outage, not a bad token) and makes no fetch;
//! - after a **successful** fetch a miss is still `BadSignature` (#660's table);
//! - the first load of an empty cache is never held back.
//!
//! Written before the implementation by the ticket author (claude/01a3d413). Do not edit, delete or
//! weaken these tests; add your own in another file. #660's `token_verifier_refetch_test.rs` and
//! CI-2's `token_verifier_test.rs` stay as they are.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::BoxFuture;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use xc_auth::domain::token::{Jwk, Jwks, JwksSource, TokenError, TokenVerifier};
use xc_auth::infrastructure::token::{JwksTokenVerifier, RefetchClock};

const ISS: &str = "auth-service";
const AUD: &str = "xchange";
const WINDOW: Duration = Duration::from_secs(60);

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct Key {
    kid: &'static str,
    pair: Ed25519KeyPair,
}

fn key(kid: &'static str, seed: u8) -> Key {
    Key {
        kid,
        pair: Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("seed"),
    }
}

fn jwk(k: &Key) -> Jwk {
    Jwk {
        kid: k.kid.to_owned(),
        kty: "OKP".to_owned(),
        crv: "Ed25519".to_owned(),
        x: URL_SAFE_NO_PAD.encode(k.pair.public_key().as_ref()),
    }
}

fn b64(v: &Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).unwrap())
}

/// A valid EdDSA token signed by `signer` under `kid`.
fn token(signer: &Key, kid: &str) -> String {
    let head = b64(&json!({ "alg": "EdDSA", "typ": "JWT", "kid": kid }));
    let body = b64(&json!({ "sub": "s", "iss": ISS, "aud": AUD, "exp": now_secs() + 600 }));
    let input = format!("{head}.{body}");
    let sig = URL_SAFE_NO_PAD.encode(signer.pair.sign(input.as_bytes()).as_ref());
    format!("{input}.{sig}")
}

/// A monotonic clock the test moves by hand.
#[derive(Default)]
struct FakeClock {
    millis: AtomicU64,
}

impl FakeClock {
    fn advance(&self, d: Duration) {
        self.millis
            .fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

impl RefetchClock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
}

/// Serves `keys` and counts every fetch attempt; while `down` is set, every attempt fails as an
/// unreachable Identity does (`HttpJwksSource` answers `KeysUnavailable`).
struct Source {
    keys: Mutex<Vec<Jwk>>,
    fetches: AtomicUsize,
    down: AtomicBool,
}

impl Source {
    fn new(keys: Vec<Jwk>) -> Arc<Self> {
        Arc::new(Self {
            keys: Mutex::new(keys),
            fetches: AtomicUsize::new(0),
            down: AtomicBool::new(false),
        })
    }
    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }
}

impl JwksSource for Source {
    fn fetch(&self) -> BoxFuture<'_, Result<Jwks, TokenError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                return Err(TokenError::KeysUnavailable);
            }
            Ok(Jwks {
                keys: self.keys.lock().unwrap().clone(),
            })
        })
    }
}

fn verifier(src: Arc<Source>, clock: Arc<FakeClock>) -> JwksTokenVerifier {
    JwksTokenVerifier::with_refetch(src, ISS, AUD, WINDOW, clock)
}

/// Loads the cache with `known`, then lets the window from that load expire.
async fn loaded_then_window_expired(
    known: &Key,
    src: &Arc<Source>,
    clock: &Arc<FakeClock>,
    v: &JwksTokenVerifier,
) {
    v.verify(&token(known, known.kid))
        .await
        .expect("the first load verifies a known key");
    assert_eq!(src.fetches(), 1, "the first load fetched once");
    clock.advance(WINDOW);
}

/// The core of #689: once a refetch has failed, more forged `kid`s inside the window make no
/// outbound request at all, however many arrive.
#[tokio::test]
async fn g1_a_failed_refetch_starts_the_window_so_forged_kids_inside_it_fetch_nothing() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    loaded_then_window_expired(&known, &src, &clock, &v).await;

    src.set_down(true);
    assert_eq!(
        v.verify(&token(&forger, "random-0")).await.unwrap_err(),
        TokenError::KeysUnavailable,
        "the refetch itself failed"
    );
    assert_eq!(
        src.fetches(),
        2,
        "exactly one refetch attempt after the window"
    );

    for n in 1..50 {
        let kid = format!("random-{n}");
        let _ = v.verify(&token(&forger, &kid)).await;
        clock.advance(Duration::from_millis(500));
    }
    assert_eq!(
        src.fetches(),
        2,
        "49 more forged kids within 24.5 s of the failed attempt sent no request to the Identity"
    );
}

/// The ruling's answer inside that window: `KeysUnavailable`, never `BadSignature`, because after a
/// failed fetch the verifier cannot tell a forged `kid` from a rotated one.
#[tokio::test]
async fn g2_inside_the_window_after_a_failed_refetch_an_unknown_kid_is_keys_unavailable() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    loaded_then_window_expired(&known, &src, &clock, &v).await;

    src.set_down(true);
    let _ = v.verify(&token(&forger, "first")).await;
    let attempts = src.fetches();

    // The Identity comes back, but the window from the failed attempt has not passed: still no fetch.
    src.set_down(false);
    clock.advance(WINDOW - Duration::from_millis(1));
    assert_eq!(
        v.verify(&token(&forger, "second")).await.unwrap_err(),
        TokenError::KeysUnavailable,
        "a miss inside the window after a failed attempt is an outage, not a bad token"
    );
    assert_eq!(src.fetches(), attempts, "and it made no request");
}

/// A known `kid` keeps verifying from the cache while the Identity is down and the window runs.
#[tokio::test]
async fn g3_a_known_kid_still_verifies_from_the_cache_after_a_failed_refetch() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    loaded_then_window_expired(&known, &src, &clock, &v).await;

    src.set_down(true);
    let _ = v.verify(&token(&forger, "random")).await;
    let attempts = src.fetches();

    v.verify(&token(&known, "env-1"))
        .await
        .expect("the cached key still verifies");
    assert_eq!(src.fetches(), attempts, "a cache hit never fetches");
}

/// The window after a failed attempt ends like any other: the first miss after it makes exactly
/// one new fetch; if that one succeeds, a key that is still absent is `BadSignature` (#660).
#[tokio::test]
async fn g4_after_the_window_following_a_failed_refetch_one_new_fetch_is_made() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    loaded_then_window_expired(&known, &src, &clock, &v).await;

    src.set_down(true);
    let _ = v.verify(&token(&forger, "random-a")).await;
    let attempts = src.fetches();

    src.set_down(false);
    clock.advance(WINDOW);
    assert_eq!(
        v.verify(&token(&forger, "random-b")).await.unwrap_err(),
        TokenError::BadSignature,
        "the fetch after the window succeeded and the kid is not in it"
    );
    assert_eq!(
        src.fetches(),
        attempts + 1,
        "exactly one fetch after the window"
    );

    // That fetch succeeded, so the new window is a successful one: a miss inside it is BadSignature
    // again, not the KeysUnavailable of the failed window before it.
    assert_eq!(
        v.verify(&token(&forger, "random-c")).await.unwrap_err(),
        TokenError::BadSignature,
        "a success clears the failed state"
    );
    assert_eq!(
        src.fetches(),
        attempts + 1,
        "and the miss inside the new window fetched nothing"
    );
}

/// After a **successful** refetch a miss inside the window is still `BadSignature` (#660's table is
/// unchanged by #689).
#[tokio::test]
async fn g5_a_miss_after_a_successful_refetch_is_still_bad_signature() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    loaded_then_window_expired(&known, &src, &clock, &v).await;

    assert_eq!(
        v.verify(&token(&forger, "random-a")).await.unwrap_err(),
        TokenError::BadSignature,
        "the refetch succeeded and the kid is not in it"
    );
    let attempts = src.fetches();
    assert_eq!(
        v.verify(&token(&forger, "random-b")).await.unwrap_err(),
        TokenError::BadSignature,
        "inside the window after a successful refetch: BadSignature, not KeysUnavailable"
    );
    assert_eq!(src.fetches(), attempts, "and no fetch");
}

/// The first load of an empty cache is never held back, even right after a failed first load: the
/// server must be able to come up while the Identity is briefly down.
#[tokio::test]
async fn g6_a_failed_first_load_does_not_hold_back_the_next_first_load() {
    let known = key("env-1", 7);
    let src = Source::new(vec![jwk(&known)]);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());

    src.set_down(true);
    assert_eq!(
        v.verify(&token(&known, "env-1")).await.unwrap_err(),
        TokenError::KeysUnavailable,
        "the first load failed"
    );
    assert_eq!(src.fetches(), 1);

    src.set_down(false);
    v.verify(&token(&known, "env-1"))
        .await
        .expect("an empty cache fetches again at once, inside any window");
    assert_eq!(src.fetches(), 2, "the second first-load attempt fetched");
}
