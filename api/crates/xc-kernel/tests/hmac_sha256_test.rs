//! Up-front tests for HMAC-SHA256 in xc-kernel (#1611).
//! Test vectors from RFC 4231 sections 4.2, 4.3, and 4.7.

use xc_kernel::pii::hmac_sha256;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Row 1: RFC 4231 test case 1 (20-byte key 0x0b ×20, data "Hi There") → RFC output.
#[test]
fn row1_rfc4231_test_case_1() {
    let key = [0x0bu8; 20];
    let data = b"Hi There";
    let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";

    let actual = hmac_sha256(&key, &[data]);
    assert_eq!(hex(&actual), expected);
}

/// Row 2: RFC 4231 test case 2 (key "Jefe", data "what do ya want for nothing?") → RFC output.
#[test]
fn row2_rfc4231_test_case_2() {
    let key = b"Jefe";
    let data = b"what do ya want for nothing?";
    let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";

    let actual = hmac_sha256(key, &[data]);
    assert_eq!(hex(&actual), expected);
}

/// Row 3: RFC 4231 test case 6 (131-byte key 0xaa ×131, data "Test Using Larger Than Block-Size Key - Hash Key First") → RFC output.
#[test]
fn row3_rfc4231_test_case_6_key_larger_than_block_size() {
    let key = [0xaau8; 131];
    let data = b"Test Using Larger Than Block-Size Key - Hash Key First";
    let expected = "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54";

    let actual = hmac_sha256(&key, &[data]);
    assert_eq!(hex(&actual), expected);
}

/// Row 4: parts is concatenation: hmac_sha256(k, &[a, b]) equals hmac_sha256(k, &[ab]).
#[test]
fn row4_parts_is_concatenation() {
    let key = b"Jefe";
    let a = b"what do ya want ";
    let b = b"for nothing?";
    let whole = b"what do ya want for nothing?";
    let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";

    let from_parts = hmac_sha256(key, &[a, b]);
    let from_whole = hmac_sha256(key, &[whole]);

    assert_eq!(hex(&from_parts), expected);
    assert_eq!(hex(&from_whole), expected);
    assert_eq!(from_parts, from_whole);
}
