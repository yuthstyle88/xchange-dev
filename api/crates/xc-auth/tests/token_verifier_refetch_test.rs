//! #660: a token with an unknown `kid` must not make the server refetch the JWKS on every request.
//! The lane controller's ruling (#660): a minimum refetch interval, 60 s by default and passed to the
//! constructor; at most one refetch in flight; an unknown `kid` inside the window fails closed with no
//! fetch; a rotated key is found by the first refetch after the window.
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken
//! these tests; add your own in another file. CI-2's tests (`token_verifier_test.rs`) stay as they are.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::BoxFuture;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use xc_auth::domain::token::{Jwk, Jwks, JwksSource, TokenError, TokenVerifier};
use xc_auth::infrastructure::token::{DEFAULT_REFETCH_INTERVAL, JwksTokenVerifier, RefetchClock};

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

/// Serves `keys`, counts every fetch, and can be slow (so fetches overlap).
struct Source {
    keys: Mutex<Vec<Jwk>>,
    fetches: AtomicUsize,
    delay: Duration,
}

impl Source {
    fn new(keys: Vec<Jwk>, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            keys: Mutex::new(keys),
            fetches: AtomicUsize::new(0),
            delay,
        })
    }
    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl JwksSource for Source {
    fn fetch(&self) -> BoxFuture<'_, Result<Jwks, TokenError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
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

#[test]
fn f1_the_default_interval_is_60_seconds() {
    assert_eq!(DEFAULT_REFETCH_INTERVAL, Duration::from_secs(60));
}

#[tokio::test]
async fn f2_unknown_kids_inside_the_window_fail_closed_without_fetching() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)], Duration::ZERO);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    v.verify(&token(&known, "env-1")).await.expect("first load");
    assert_eq!(src.fetches(), 1);
    for n in 0..100 {
        let kid = format!("random-{n}");
        assert_eq!(
            v.verify(&token(&forger, &kid)).await.unwrap_err(),
            TokenError::BadSignature
        );
    }
    assert_eq!(
        src.fetches(),
        1,
        "no refetch inside the window, however many unknown kids arrive"
    );
}

#[tokio::test]
async fn f3_the_window_is_measured_from_the_last_fetch_and_its_edge_allows_one_refetch() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)], Duration::ZERO);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    v.verify(&token(&known, "env-1")).await.unwrap();
    clock.advance(WINDOW - Duration::from_millis(1));
    assert_eq!(
        v.verify(&token(&forger, "x1")).await.unwrap_err(),
        TokenError::BadSignature
    );
    assert_eq!(src.fetches(), 1, "1 ms before the window ends: no fetch");
    clock.advance(Duration::from_millis(1));
    assert_eq!(
        v.verify(&token(&forger, "x2")).await.unwrap_err(),
        TokenError::BadSignature
    );
    assert_eq!(src.fetches(), 2, "at the window's end: exactly one refetch");
    assert_eq!(
        v.verify(&token(&forger, "x3")).await.unwrap_err(),
        TokenError::BadSignature
    );
    assert_eq!(src.fetches(), 2, "a new window started at that refetch");
}

#[tokio::test]
async fn f4_a_rotated_key_is_refused_inside_the_window_and_found_after_it() {
    let old = key("env-1", 7);
    let new = key("env-2", 8);
    let src = Source::new(vec![jwk(&old)], Duration::ZERO);
    let clock = Arc::new(FakeClock::default());
    let v = verifier(src.clone(), clock.clone());
    v.verify(&token(&old, "env-1")).await.unwrap();
    src.keys.lock().unwrap().push(jwk(&new));
    assert_eq!(
        v.verify(&token(&new, "env-2")).await.unwrap_err(),
        TokenError::BadSignature
    );
    assert_eq!(src.fetches(), 1);
    clock.advance(WINDOW);
    let got = v
        .verify(&token(&new, "env-2"))
        .await
        .expect("found by the refetch after the window");
    assert_eq!(got.subject, "s");
    assert_eq!(src.fetches(), 2);
    // Known kids keep working without fetching.
    v.verify(&token(&old, "env-1")).await.unwrap();
    v.verify(&token(&new, "env-2")).await.unwrap();
    assert_eq!(src.fetches(), 2);
}

#[tokio::test]
async fn f5_concurrent_unknown_kids_after_the_window_share_one_refetch() {
    let known = key("env-1", 7);
    let forger = key("forged", 9);
    let src = Source::new(vec![jwk(&known)], Duration::from_millis(50));
    let clock = Arc::new(FakeClock::default());
    let v = Arc::new(verifier(src.clone(), clock.clone()));
    v.verify(&token(&known, "env-1")).await.unwrap();
    clock.advance(WINDOW);
    let mut tasks = Vec::new();
    for n in 0..20 {
        let v = v.clone();
        let t = token(&forger, &format!("burst-{n}"));
        tasks.push(tokio::spawn(async move { v.verify(&t).await }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap().unwrap_err(), TokenError::BadSignature);
    }
    assert_eq!(
        src.fetches(),
        2,
        "the first load plus exactly one refetch, never one per request"
    );
}

#[tokio::test]
async fn f6_the_first_load_is_not_held_back_by_the_window() {
    let known = key("env-1", 7);
    let src = Source::new(vec![jwk(&known)], Duration::ZERO);
    let v = verifier(src.clone(), Arc::new(FakeClock::default()));
    v.verify(&token(&known, "env-1"))
        .await
        .expect("an empty cache always loads");
    assert_eq!(src.fetches(), 1);
}
