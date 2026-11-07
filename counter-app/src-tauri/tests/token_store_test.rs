//! CA-3c (design §2): the `TokenStore` contract, on `MemoryTokenStore`. The real Keychain is never
//! touched in a test. Written before the implementation. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! | Step | `load` answers |
//! |---|---|
//! | new store | `None` |
//! | after `save("a")` | `Some("a")` |
//! | after `save("b")` | `Some("b")`: the token is replaced |
//! | after `clear` | `None`; a second `clear` is fine |

use xc_counter::keychain::{MemoryTokenStore, TokenStore};

#[test]
fn memory_store_saves_replaces_and_clears() {
    let store = MemoryTokenStore::default();
    assert_eq!(store.load(), Ok(None));
    store.save("a").expect("save");
    assert_eq!(store.load(), Ok(Some("a".to_owned())));
    store.save("b").expect("save");
    assert_eq!(store.load(), Ok(Some("b".to_owned())));
    store.clear().expect("clear");
    assert_eq!(store.load(), Ok(None));
    store.clear().expect("clearing an empty store is fine");
}
