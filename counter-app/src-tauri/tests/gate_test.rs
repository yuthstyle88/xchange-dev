//! CA-3d (design §8.2, review item 3a): the release gate. Written before the implementation. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! | `GET /api/device-agent/info` | Gate | Means |
//! |---|---|---|
//! | 200 | Run | start the runner |
//! | 404 | ServerTooOld | a server older than CA-1; claims nothing |
//! | 401 | RePair | the token is unknown or revoked |
//! | 400, 403, 429, 500, 502, 503 | Retry | ask again; claims nothing meanwhile |

use xc_counter::agent::{Gate, gate};

#[test]
fn ok_runs() {
    assert_eq!(gate(200), Gate::Run);
}

#[test]
fn not_found_means_the_server_is_too_old() {
    assert_eq!(gate(404), Gate::ServerTooOld);
}

#[test]
fn unauthorized_means_re_pair() {
    assert_eq!(gate(401), Gate::RePair);
}

#[test]
fn anything_else_retries() {
    for status in [400, 403, 429, 500, 502, 503] {
        assert_eq!(gate(status), Gate::Retry, "{status}");
    }
}
