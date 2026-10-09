//! CI-2 (#649): I-2, the token verifier. Design `docs/design/blueprint/customer-identity-consent.md` §2 K9,
//! §4.5; #550 `org-roles-branches.md` §5.2 (OR-3 verifies through this, it does not build a second one).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! Tokens are minted here with `ring`'s Ed25519 (already in `Cargo.lock`), so no network and no Identity
//! instance is needed: the verifier gets its keys from an injected `JwksSource`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::BoxFuture;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};
use std::sync::Mutex;

use xc_auth::domain::token::{Jwk, Jwks, JwksSource, TokenError, TokenVerifier, VerifiedToken};
use xc_auth::infrastructure::token::JwksTokenVerifier;

const ISS: &str = "auth-service";
const AUD: &str = "xchange";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct Key {
    kid: &'static str,
    pair: Ed25519KeyPair,
}

fn key(kid: &'static str, seed_byte: u8) -> Key {
    Key {
        kid,
        pair: Ed25519KeyPair::from_seed_unchecked(&[seed_byte; 32]).expect("seed"),
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

/// A compact JWS signed by `signer`, with `kid` and `alg` in the header as given.
fn mint(signer: &Key, kid: &str, alg: &str, claims: Value) -> String {
    let head = b64(&json!({ "alg": alg, "typ": "JWT", "kid": kid }));
    let body = b64(&claims);
    let input = format!("{head}.{body}");
    let sig = URL_SAFE_NO_PAD.encode(signer.pair.sign(input.as_bytes()).as_ref());
    format!("{input}.{sig}")
}

fn claims(sub: &str, exp: i64) -> Value {
    json!({ "sub": sub, "iss": ISS, "aud": AUD, "iat": now(), "exp": exp })
}

/// Serves `keys`, counts every fetch, and can be told to fail or to change its keys.
struct FakeSource {
    keys: Mutex<Vec<Jwk>>,
    fetches: AtomicUsize,
    fail: bool,
}

impl FakeSource {
    fn with(keys: Vec<Jwk>) -> Arc<Self> {
        Arc::new(Self {
            keys: Mutex::new(keys),
            fetches: AtomicUsize::new(0),
            fail: false,
        })
    }
    fn failing() -> Arc<Self> {
        Arc::new(Self {
            keys: Mutex::new(vec![]),
            fetches: AtomicUsize::new(0),
            fail: true,
        })
    }
    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl JwksSource for FakeSource {
    fn fetch(&self) -> BoxFuture<'_, Result<Jwks, TokenError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(TokenError::KeysUnavailable);
            }
            Ok(Jwks {
                keys: self.keys.lock().unwrap().clone(),
            })
        })
    }
}

fn verifier(source: Arc<FakeSource>) -> JwksTokenVerifier {
    JwksTokenVerifier::new(source, ISS, AUD)
}

// ---- accepted --------------------------------------------------------------------------------

#[tokio::test]
async fn v1_a_token_signed_by_a_jwks_key_with_the_right_iss_aud_and_exp_verifies() {
    let k = key("env-1", 7);
    let exp = now() + 600;
    let v = verifier(FakeSource::with(vec![jwk(&k)]));
    let got: VerifiedToken = v
        .verify(&mint(&k, "env-1", "EdDSA", claims("user-abc", exp)))
        .await
        .expect("valid");
    assert_eq!(got.subject, "user-abc");
    assert_eq!(got.expires_at, exp);
    assert_eq!(got.phone, None, "no phone claim → None");
}

#[tokio::test]
async fn v2_a_phone_claim_is_carried() {
    let k = key("env-1", 7);
    let mut c = claims("user-abc", now() + 600);
    c["phone"] = json!("+66812345678");
    let got = verifier(FakeSource::with(vec![jwk(&k)]))
        .verify(&mint(&k, "env-1", "EdDSA", c))
        .await
        .unwrap();
    assert_eq!(got.phone.as_deref(), Some("+66812345678"));
}

#[tokio::test]
async fn v3_an_aud_array_that_contains_xchange_is_accepted() {
    let k = key("env-1", 7);
    let mut c = claims("user-abc", now() + 600);
    c["aud"] = json!(["other", AUD]);
    assert!(
        verifier(FakeSource::with(vec![jwk(&k)]))
            .verify(&mint(&k, "env-1", "EdDSA", c))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn v4_keys_are_fetched_once_and_reused() {
    let k = key("env-1", 7);
    let src = FakeSource::with(vec![jwk(&k)]);
    let v = verifier(src.clone());
    for sub in ["a", "b", "c"] {
        v.verify(&mint(&k, "env-1", "EdDSA", claims(sub, now() + 600)))
            .await
            .unwrap();
    }
    assert_eq!(src.fetches(), 1, "a known kid is served from the cache");
}

// ---- refused ---------------------------------------------------------------------------------

#[tokio::test]
async fn r1_a_signature_by_another_key_under_the_same_kid_is_bad_signature() {
    let real = key("env-1", 7);
    let forger = key("env-1", 9);
    let v = verifier(FakeSource::with(vec![jwk(&real)]));
    let err = v
        .verify(&mint(&forger, "env-1", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::BadSignature);
}

#[tokio::test]
async fn r2_a_tampered_payload_is_bad_signature() {
    let k = key("env-1", 7);
    let t = mint(&k, "env-1", "EdDSA", claims("user-abc", now() + 600));
    let parts: Vec<&str> = t.split('.').collect();
    let forged_body = b64(&claims("someone-else", now() + 600));
    let forged = format!("{}.{}.{}", parts[0], forged_body, parts[2]);
    let err = verifier(FakeSource::with(vec![jwk(&k)]))
        .verify(&forged)
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::BadSignature);
}

#[tokio::test]
async fn r3_a_wrong_issuer_is_refused() {
    let k = key("env-1", 7);
    let mut c = claims("x", now() + 600);
    c["iss"] = json!("someone-else");
    let err = verifier(FakeSource::with(vec![jwk(&k)]))
        .verify(&mint(&k, "env-1", "EdDSA", c))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::WrongIssuer);
}

#[tokio::test]
async fn r4_a_wrong_audience_is_refused() {
    let k = key("env-1", 7);
    let mut c = claims("x", now() + 600);
    c["aud"] = json!("pos108");
    let err = verifier(FakeSource::with(vec![jwk(&k)]))
        .verify(&mint(&k, "env-1", "EdDSA", c))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::WrongAudience);
}

#[tokio::test]
async fn r5_an_expired_token_is_refused() {
    let k = key("env-1", 7);
    // Two minutes past: beyond any clock leeway the implementation may allow (at most 60 s).
    let err = verifier(FakeSource::with(vec![jwk(&k)]))
        .verify(&mint(&k, "env-1", "EdDSA", claims("x", now() - 120)))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::Expired);
}

#[tokio::test]
async fn r6_an_alg_other_than_eddsa_is_malformed_even_with_a_valid_ed25519_signature() {
    let k = key("env-1", 7);
    let v = verifier(FakeSource::with(vec![jwk(&k)]));
    for alg in ["HS256", "none", "RS256", "ES256"] {
        let err = v
            .verify(&mint(&k, "env-1", alg, claims("x", now() + 600)))
            .await
            .unwrap_err();
        assert_eq!(err, TokenError::Malformed, "alg {alg}");
    }
}

#[tokio::test]
async fn r7_garbage_is_malformed() {
    let v = verifier(FakeSource::with(vec![]));
    for jwt in ["", "abc", "a.b", "a.b.c.d", "!!!.###.$$$"] {
        assert_eq!(
            v.verify(jwt).await.unwrap_err(),
            TokenError::Malformed,
            "{jwt:?}"
        );
    }
}

#[tokio::test]
async fn r8_a_token_without_sub_or_exp_is_malformed() {
    let k = key("env-1", 7);
    let v = verifier(FakeSource::with(vec![jwk(&k)]));
    let no_sub = json!({ "iss": ISS, "aud": AUD, "exp": now() + 600 });
    let no_exp = json!({ "sub": "x", "iss": ISS, "aud": AUD });
    assert_eq!(
        v.verify(&mint(&k, "env-1", "EdDSA", no_sub))
            .await
            .unwrap_err(),
        TokenError::Malformed
    );
    assert_eq!(
        v.verify(&mint(&k, "env-1", "EdDSA", no_exp))
            .await
            .unwrap_err(),
        TokenError::Malformed
    );
}

#[tokio::test]
async fn r9_a_jwk_that_is_not_okp_ed25519_is_ignored() {
    let k = key("env-1", 7);
    let mut wrong = jwk(&k);
    wrong.crv = "X25519".to_owned();
    let err = verifier(FakeSource::with(vec![wrong]))
        .verify(&mint(&k, "env-1", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::BadSignature);
}

// ---- unknown kid and an unreachable source ---------------------------------------------------

#[tokio::test]
async fn u1_an_unknown_kid_refetches_once_then_is_bad_signature() {
    let known = key("env-1", 7);
    let stranger = key("env-2", 8);
    let src = FakeSource::with(vec![jwk(&known)]);
    let v = verifier(src.clone());
    v.verify(&mint(&known, "env-1", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap();
    assert_eq!(src.fetches(), 1);
    let err = v
        .verify(&mint(&stranger, "env-2", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::BadSignature);
    assert_eq!(src.fetches(), 2, "exactly one refetch for the unknown kid");
}

#[tokio::test]
async fn u2_a_rotated_key_is_found_by_the_refetch() {
    let old = key("env-1", 7);
    let new = key("env-2", 8);
    let src = FakeSource::with(vec![jwk(&old)]);
    let v = verifier(src.clone());
    v.verify(&mint(&old, "env-1", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap();
    src.keys.lock().unwrap().push(jwk(&new));
    let got = v
        .verify(&mint(&new, "env-2", "EdDSA", claims("y", now() + 600)))
        .await
        .expect("rotated key");
    assert_eq!(got.subject, "y");
}

#[tokio::test]
async fn u3_an_unreachable_source_is_keys_unavailable() {
    let k = key("env-1", 7);
    let err = verifier(FakeSource::failing())
        .verify(&mint(&k, "env-1", "EdDSA", claims("x", now() + 600)))
        .await
        .unwrap_err();
    assert_eq!(err, TokenError::KeysUnavailable);
}
