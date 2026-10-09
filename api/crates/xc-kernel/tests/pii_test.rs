//! CI-3 (#554): the PII core in `xc-kernel::pii`. Design `docs/design/blueprint/customer-identity-consent.md`
//! §2 K2/K3, §4.1 "Encrypted payload", "Normalisation" and "Keys".
//!
//! Written before the implementation (drafted by claude/b060e9; provided by the ticket author claude/89458e53, #650). Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! Known-answer vectors were computed outside Rust (Python `hmac`/`hashlib`, RFC 5869 HKDF with an
//! empty salt), so they check the construction, not just self-consistency.

use std::sync::Arc;

use xc_kernel::pii::{
    IdentityType, LookupHasher, Masked, PiiCipher, PiiError, PiiKeys, Sealed, normalize,
};

/// 32 bytes 0x00..0x1f and 0x20..0x3f, base64.
const K1: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const K2: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";

fn one_key() -> PiiKeys {
    PiiKeys::parse(&format!("k1:{K1}\n")).expect("one key parses")
}

fn two_keys() -> PiiKeys {
    PiiKeys::parse(&format!("# comment\nk1:{K1}\n\nk2:{K2}\n")).expect("two keys parse")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- key file -------------------------------------------------------------------------------

#[test]
fn k1_the_last_line_is_the_newest_key_and_encrypts() {
    assert_eq!(two_keys().current_id(), "k2");
    assert_eq!(one_key().current_id(), "k1");
}

#[test]
fn k2_blank_lines_and_hash_comments_are_ignored() {
    let keys = PiiKeys::parse(&format!("\n# a\n  \nk1:{K1}\n# b\n")).unwrap();
    assert_eq!(keys.current_id(), "k1");
}

#[test]
fn k3_a_bad_key_file_is_refused() {
    for bad in [
        String::new(),                    // no key at all
        "# only a comment\n".to_string(), // no key at all
        "k1\n".to_string(),               // no ':'
        "k1:not-base64!!\n".to_string(),  // not base64
        "k1:AAEC\n".to_string(),          // not 32 bytes
        format!("k1:{K1}\nk1:{K2}\n"),    // duplicate id
        format!(":{K1}\n"),               // empty id
    ] {
        assert!(
            matches!(PiiKeys::parse(&bad), Err(PiiError::KeyFile(_))),
            "refused: {bad:?}"
        );
    }
}

#[test]
fn k4_load_reads_the_file_and_a_missing_file_is_an_error() {
    let dir = std::env::temp_dir().join(format!("xc-pii-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pii.key");
    std::fs::write(&path, format!("k1:{K1}\n")).unwrap();
    assert_eq!(PiiKeys::load(&path).unwrap().current_id(), "k1");
    assert!(matches!(
        PiiKeys::load(&dir.join("absent.key")),
        Err(PiiError::KeyFile(_))
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn k5_debug_never_prints_key_material() {
    let keys = two_keys();
    let dbg = format!("{keys:?}");
    assert!(!dbg.contains(K1) && !dbg.contains(K2), "{dbg}");
    assert!(!dbg.contains("0, 1, 2, 3"), "raw bytes leaked: {dbg}");
}

// ---- cipher ---------------------------------------------------------------------------------

#[test]
fn c1_encrypt_then_decrypt_round_trips() {
    let c = PiiCipher::new(Arc::new(one_key()));
    let sealed = c.encrypt(b"CX-00000001|PASSPORT", b"{\"number\":\"AA1234567\"}");
    assert_eq!(sealed.key_id, "k1");
    assert_eq!(
        c.decrypt(b"CX-00000001|PASSPORT", &sealed).unwrap(),
        b"{\"number\":\"AA1234567\"}"
    );
}

#[test]
fn c2_the_ciphertext_is_not_the_plain_text_and_two_encryptions_differ() {
    let c = PiiCipher::new(Arc::new(one_key()));
    let a = c.encrypt(b"aad", b"AA1234567");
    let b = c.encrypt(b"aad", b"AA1234567");
    assert_ne!(a.bytes, b.bytes, "a fresh nonce per encryption");
    assert!(
        !a.bytes.windows(9).any(|w| w == b"AA1234567"),
        "plain text visible"
    );
    assert_eq!(
        a.bytes.len(),
        12 + 9 + 16,
        "nonce (12) || ciphertext || tag (16)"
    );
}

#[test]
fn c3_a_changed_aad_or_a_flipped_byte_fails() {
    let c = PiiCipher::new(Arc::new(one_key()));
    let s = c.encrypt(b"CX-1|PASSPORT", b"secret");
    assert!(matches!(
        c.decrypt(b"CX-2|PASSPORT", &s),
        Err(PiiError::Decrypt)
    ));
    let mut bad = s.clone();
    let last = bad.bytes.len() - 1;
    bad.bytes[last] ^= 1;
    assert!(matches!(
        c.decrypt(b"CX-1|PASSPORT", &bad),
        Err(PiiError::Decrypt)
    ));
    let short = Sealed {
        key_id: "k1".into(),
        bytes: vec![0; 5],
    };
    assert!(matches!(
        c.decrypt(b"CX-1|PASSPORT", &short),
        Err(PiiError::Decrypt)
    ));
}

#[test]
fn c4_the_newest_key_encrypts_and_an_older_one_still_decrypts() {
    let old = PiiCipher::new(Arc::new(one_key()));
    let sealed_by_k1 = old.encrypt(b"aad", b"value");
    let rotated = PiiCipher::new(Arc::new(two_keys()));
    assert_eq!(rotated.encrypt(b"aad", b"value").key_id, "k2");
    assert_eq!(rotated.decrypt(b"aad", &sealed_by_k1).unwrap(), b"value");
}

#[test]
fn c5_an_unknown_key_id_is_its_own_error() {
    let c = PiiCipher::new(Arc::new(one_key()));
    let mut s = c.encrypt(b"aad", b"value");
    s.key_id = "k9".into();
    assert!(matches!(c.decrypt(b"aad", &s), Err(PiiError::UnknownKey(id)) if id == "k9"));
}

// ---- lookup hash ----------------------------------------------------------------------------

#[test]
fn h1_known_answer_hkdf_from_the_oldest_key_then_hmac_of_type_unit_separator_value() {
    // hash key = HKDF-SHA256(ikm = K1, salt = none, info = "xc-pii-lookup-v1"), 32 bytes
    // hash = HMAC-SHA256(hash key, "<TYPE>" || 0x1f || normalized)
    let h = LookupHasher::new(&one_key());
    assert_eq!(
        hex(&h.hash(IdentityType::Passport, "PASSPORT:THA:AA1234567")),
        "86a6f1be093ae1f8c61cccb7a5211f8a07080d780cbc62c0b379f8aa00b694ff"
    );
    assert_eq!(
        hex(&h.hash(IdentityType::Email, "a@b.co")),
        "0bf28278f382123817c2e4e2ab31cc949be3dee16b1c9423da58a4cb0e539b31"
    );
}

#[test]
fn h2_the_hash_is_stable_across_encryption_key_rotation() {
    let before =
        LookupHasher::new(&one_key()).hash(IdentityType::Passport, "PASSPORT:THA:AA1234567");
    let after =
        LookupHasher::new(&two_keys()).hash(IdentityType::Passport, "PASSPORT:THA:AA1234567");
    assert_eq!(
        before, after,
        "the hash key comes from the oldest key, not the newest"
    );
}

#[test]
fn h3_the_type_is_part_of_the_hash() {
    let h = LookupHasher::new(&one_key());
    assert_ne!(
        h.hash(IdentityType::Phone, "+66812345678"),
        h.hash(IdentityType::NationalId, "+66812345678")
    );
}

#[test]
fn h4_a_different_oldest_key_gives_a_different_hash() {
    let other = PiiKeys::parse(&format!("k2:{K2}\n")).unwrap();
    assert_ne!(
        LookupHasher::new(&one_key()).hash(IdentityType::Email, "a@b.co"),
        LookupHasher::new(&other).hash(IdentityType::Email, "a@b.co"),
    );
}

// ---- normalisation --------------------------------------------------------------------------

#[test]
fn n1_passport_upper_case_no_spaces_no_dashes_with_nationality() {
    for raw in ["aa1234567", " AA 123-4567 ", "Aa-1234567"] {
        assert_eq!(
            normalize(IdentityType::Passport, raw, Some("tha")).unwrap(),
            "PASSPORT:THA:AA1234567",
            "{raw:?}"
        );
    }
}

#[test]
fn n2_passport_needs_a_three_letter_nationality_and_a_number() {
    assert!(matches!(
        normalize(IdentityType::Passport, "AA1234567", None),
        Err(PiiError::InvalidNationality)
    ));
    assert!(matches!(
        normalize(IdentityType::Passport, "AA1234567", Some("TH")),
        Err(PiiError::InvalidNationality)
    ));
    assert!(matches!(
        normalize(IdentityType::Passport, "AA1234567", Some("T1A")),
        Err(PiiError::InvalidNationality)
    ));
    assert!(matches!(
        normalize(IdentityType::Passport, " - ", Some("THA")),
        Err(PiiError::Empty)
    ));
}

#[test]
fn n3_national_id_keeps_digits_only() {
    assert_eq!(
        normalize(IdentityType::NationalId, "1-2345-67890-12-3", None).unwrap(),
        "1234567890123"
    );
    assert!(matches!(
        normalize(IdentityType::NationalId, "--", None),
        Err(PiiError::Empty)
    ));
}

#[test]
fn n4_a_thai_phone_written_with_0_becomes_plus_66() {
    assert_eq!(
        normalize(IdentityType::Phone, "081-234-5678", None).unwrap(),
        "+66812345678"
    );
    assert_eq!(
        normalize(IdentityType::Phone, "(081) 234 5678", None).unwrap(),
        "+66812345678"
    );
    assert_eq!(
        normalize(IdentityType::Phone, "+66 81 234 5678", None).unwrap(),
        "+66812345678"
    );
    assert_eq!(
        normalize(IdentityType::Phone, "+44 20 7946 0958", None).unwrap(),
        "+442079460958"
    );
}

#[test]
fn n5_a_phone_that_is_neither_0_nor_plus_is_refused() {
    assert!(matches!(
        normalize(IdentityType::Phone, "812345678", None),
        Err(PiiError::InvalidPhone)
    ));
    assert!(matches!(
        normalize(IdentityType::Phone, "+66 81x", None),
        Err(PiiError::InvalidPhone)
    ));
    assert!(matches!(
        normalize(IdentityType::Phone, "  ", None),
        Err(PiiError::Empty)
    ));
}

#[test]
fn n6_email_is_trimmed_and_lower_cased() {
    assert_eq!(
        normalize(IdentityType::Email, "  A.Person@Example.COM ", None).unwrap(),
        "a.person@example.com"
    );
    assert!(matches!(
        normalize(IdentityType::Email, "no-at-sign", None),
        Err(PiiError::InvalidEmail)
    ));
    assert!(matches!(
        normalize(IdentityType::Email, " ", None),
        Err(PiiError::Empty)
    ));
}

#[test]
fn n7_the_same_passport_spelt_differently_hashes_the_same() {
    let h = LookupHasher::new(&one_key());
    let a = normalize(IdentityType::Passport, "aa 123-4567", Some("tha")).unwrap();
    let b = normalize(IdentityType::Passport, "AA1234567", Some("THA")).unwrap();
    assert_eq!(
        h.hash(IdentityType::Passport, &a),
        h.hash(IdentityType::Passport, &b)
    );
}

#[test]
fn n8_identity_type_names_are_the_stored_ones() {
    assert_eq!(IdentityType::Passport.as_str(), "PASSPORT");
    assert_eq!(IdentityType::NationalId.as_str(), "NATIONAL_ID");
    assert_eq!(IdentityType::Phone.as_str(), "PHONE");
    assert_eq!(IdentityType::Email.as_str(), "EMAIL");
}

// ---- masked ---------------------------------------------------------------------------------

#[test]
fn m1_masked_shows_only_the_last_four_after_a_fixed_prefix() {
    assert_eq!(Masked::of("AA1234567").to_string(), "****4567");
    assert_eq!(Masked::of("+66812345678").to_string(), "****5678");
}

#[test]
fn m2_four_or_fewer_characters_show_nothing() {
    assert_eq!(Masked::of("1234").to_string(), "****");
    assert_eq!(Masked::of("").to_string(), "****");
}

#[test]
fn m3_masked_debug_is_its_display() {
    assert_eq!(format!("{:?}", Masked::of("AA1234567")), "****4567");
}

#[test]
fn m4_masking_counts_characters_not_bytes() {
    assert_eq!(Masked::of("กขคงจฉ").to_string(), "****คงจฉ");
}
