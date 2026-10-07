//! Written before the implementation (ticket #163). Do not edit, delete or weaken.
//!
//! The one shape of a branch name: 1 to 70 characters of `A-Za-z0-9-`. This is v2's
//! `/^[A-Za-z0-9-]{1,70}$/` (`server.mjs:6999`) and what every copy on `main` checks today
//! (#163 lists them). Whether such a branch exists is not this rule's question.

use xc_kernel::ids::BranchId;

#[test]
fn names_of_the_right_shape_are_well_formed() {
    let seventy = "A".repeat(70);
    for name in [
        "MAIN",
        "BR-2",
        "B1a2b3",
        "a-b",
        "-",
        "0",
        "z",
        "a--b",
        seventy.as_str(),
    ] {
        assert!(BranchId::is_well_formed(name), "{name:?} is well formed");
    }
}

#[test]
fn names_of_any_other_shape_are_not() {
    let seventy_one = "A".repeat(71);
    let two_hundred = "b".repeat(200);
    for name in [
        "",
        seventy_one.as_str(),
        two_hundred.as_str(),
        "MAIN_1",
        "a b",
        " MAIN",
        "MAIN ",
        "a.b",
        "a/b",
        "%00",
        "\0",
        "MAIN\n",
        "\t",
        "ก",
        "é",
        "Ａ",
        "١",
        "MAIN;",
        "b+1",
    ] {
        assert!(
            !BranchId::is_well_formed(name),
            "{name:?} is not well formed"
        );
    }
}

#[test]
fn the_length_counts_characters_of_the_allowed_alphabet_only() {
    // 69 letters and one Thai character is 70 characters but not well formed
    let mixed = format!("{}ก", "A".repeat(69));
    assert!(
        !BranchId::is_well_formed(&mixed),
        "a non-ASCII character is never allowed"
    );
    // 70 hyphens is 70 characters of the alphabet
    assert!(BranchId::is_well_formed(&"-".repeat(70)));
}

#[test]
fn the_constructor_still_checks_nothing() {
    // `BranchId::new` stays a plain wrapper: ids read back from the database are not re-checked
    assert_eq!(BranchId::new("MAIN_1").as_str(), "MAIN_1");
    assert_eq!(BranchId::new("").as_str(), "");
}
