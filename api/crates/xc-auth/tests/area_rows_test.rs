//! PL-0b #1014 (`docs/design/blueprint/broker-seam.md` §11.1): the guard every area file is
//! checked by. Two area files that name one `(path, action)`, or an area row that overrides a row
//! `need_table` already answers, fail the required `api` check here, because the build script
//! cannot evaluate `EntryPath` values.
//!
//! Written by the ticket author before the implementation. Do not edit, delete or weaken.

use xc_auth::domain::policy::areas::{AREA_ROWS, duplicates, shadowed};

#[test]
fn no_path_and_action_is_named_by_two_area_rows() {
    assert_eq!(
        duplicates(AREA_ROWS),
        vec![],
        "two area rows name the same (path, action)"
    );
}

#[test]
fn no_area_row_overrides_a_row_of_the_route_table() {
    assert_eq!(
        shadowed(AREA_ROWS),
        vec![],
        "an area row names a (path, action) that need_table already answers; change the table row instead"
    );
}
