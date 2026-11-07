//! XC Counter §5.2 and §8.2: a counter with no drawer claims with no `drawer` key, and a counter
//! with one sends the same bytes as before. Written before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! | drawer | JSON of the claim |
//! |---|---|
//! | `None` | no `drawer` key at all |
//! | `Some` | today's JSON, byte for byte, key order included |

use xc_device_agent::client::{ClaimRequest, DrawerInfo};

fn claim(drawer: Option<DrawerInfo>) -> ClaimRequest {
    ClaimRequest {
        wait_ms: 20_000,
        version: "1.0.0".to_owned(),
        build: "abc1234".to_owned(),
        os: "macos".to_owned(),
        host: "counter-1".to_owned(),
        drawer,
    }
}

#[test]
fn claim_with_no_drawer_has_no_drawer_key() {
    let json = serde_json::to_string(&claim(None)).expect("serialises");
    assert_eq!(
        json,
        r#"{"waitMs":20000,"version":"1.0.0","build":"abc1234","os":"macos","host":"counter-1"}"#
    );
}

#[test]
fn claim_with_a_drawer_keeps_todays_json_byte_for_byte() {
    let json = serde_json::to_string(&claim(Some(DrawerInfo {
        transport: "CUPS".to_owned(),
        target: "POS80".to_owned(),
    })))
    .expect("serialises");
    assert_eq!(
        json,
        r#"{"waitMs":20000,"version":"1.0.0","build":"abc1234","os":"macos","host":"counter-1","drawer":{"transport":"CUPS","target":"POS80"}}"#
    );
}
