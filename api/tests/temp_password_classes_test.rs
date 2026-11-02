//! A temporary password always holds a digit, an upper-case and a lower-case letter, so Identity's password policy
//! never refuses it (#564 issuecomment-6566739711: staff create answered `422 weakPassword` and left the user
//! PENDING). Design `docs/design/blueprint/user-management-rbac.md` §3.4 (16 characters, letters and digits, no
//! look-alikes) still holds; `rbac_c1a_temp_password_test.rs` keeps pinning it.
//!
//! Root cause pinned here: `TempPassword::generate` drew 16 characters from 8 digits and 48 letters with no class
//! guaranteed, so (48/56)^16 ≈ 8.5% of passwords had no digit.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests
//! in a file of your own. No database.

use xc_auth::domain::user::TempPassword;

const DRAWS: usize = 20_000;
const ALPHABET: &str = "23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz";

/// Row 1: every draw holds at least one digit, one upper-case and one lower-case letter.
#[test]
fn every_temporary_password_holds_a_digit_an_upper_and_a_lower_letter() {
    let mut missing = (0, 0, 0);
    for _ in 0..DRAWS {
        let p = TempPassword::generate();
        let s = p.expose();
        if !s.chars().any(|c| c.is_ascii_digit()) {
            missing.0 += 1;
        }
        if !s.chars().any(|c| c.is_ascii_uppercase()) {
            missing.1 += 1;
        }
        if !s.chars().any(|c| c.is_ascii_lowercase()) {
            missing.2 += 1;
        }
    }
    assert_eq!(
        missing,
        (0, 0, 0),
        "(no digit, no upper, no lower) counts in {DRAWS} draws"
    );
}

/// Row 2: the length and the alphabet do not change (16 characters from the same 56).
#[test]
fn the_length_and_the_alphabet_stay_as_they_are() {
    for _ in 0..DRAWS {
        let p = TempPassword::generate();
        assert_eq!(p.expose().len(), 16);
        for c in p.expose().chars() {
            assert!(ALPHABET.contains(c), "{c:?} is outside the alphabet");
        }
    }
}

/// Row 3: no position is reserved for a class. A fix that always puts the digit (or a letter) in a fixed place
/// makes part of the password predictable; every position must hold both digits and letters across the draws.
#[test]
fn no_position_is_reserved_for_one_class() {
    let mut digit_at = [0usize; 16];
    let mut letter_at = [0usize; 16];
    for _ in 0..DRAWS {
        let p = TempPassword::generate();
        for (i, c) in p.expose().chars().enumerate() {
            if c.is_ascii_digit() {
                digit_at[i] += 1;
            } else {
                letter_at[i] += 1;
            }
        }
    }
    // Uniform over the alphabet, a position holds a digit 8/56 ≈ 14% of the time; allow a wide band.
    for i in 0..16 {
        let share = digit_at[i] as f64 / DRAWS as f64;
        assert!(
            (0.08..=0.25).contains(&share),
            "position {i}: digit share {share:.3} (digits {}, letters {})",
            digit_at[i],
            letter_at[i]
        );
    }
}
