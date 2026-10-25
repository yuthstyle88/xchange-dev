use xc_kernel::notification_params::{DisplayValue, DisplayValueError};

#[test]
fn server_id_accepts_one_char() {
    for id in ["a", "Z", "0", "9", "_", "-"] {
        let dv =
            DisplayValue::server_id(id).expect("1-char ASCII alphanumeric, _ or - is accepted");
        assert_eq!(dv.as_str(), id);
    }
}

#[test]
fn server_id_accepts_sixty_four_chars() {
    let id_64 = "a".repeat(64);
    let dv = DisplayValue::server_id(&id_64).expect("64-char id is accepted");
    assert_eq!(dv.as_str(), id_64);

    let mix_64 = "aB0_-".repeat(12) + "abcd";
    assert_eq!(mix_64.len(), 64);
    let dv_mix = DisplayValue::server_id(&mix_64).expect("64-char mixed id is accepted");
    assert_eq!(dv_mix.as_str(), mix_64);
}

#[test]
fn server_id_accepts_mix_of_letters_digits_underscore_and_hyphen() {
    for id in [
        "main_58ff6b53-4abc_123",
        "SH-12345_branch-01",
        "00000000-0000-0000-0000-000000000001",
        "user_role-admin_99",
    ] {
        let dv = DisplayValue::server_id(id).expect("mixed id is accepted");
        assert_eq!(dv.as_str(), id);
    }
}

#[test]
fn server_id_refuses_empty() {
    assert_eq!(
        DisplayValue::server_id(""),
        Err(DisplayValueError::ServerId("".to_string()))
    );
}

#[test]
fn server_id_refuses_sixty_five_chars() {
    let id_65 = "a".repeat(65);
    assert_eq!(
        DisplayValue::server_id(&id_65),
        Err(DisplayValueError::ServerId(id_65))
    );
}

#[test]
fn server_id_refuses_spaces() {
    for id in [" ", "a b", " main", "main ", "a  b"] {
        assert_eq!(
            DisplayValue::server_id(id),
            Err(DisplayValueError::ServerId(id.to_string()))
        );
    }
}

#[test]
fn server_id_refuses_punctuation() {
    for id in [
        "a:b",
        "a/b",
        "a.b",
        "a@b",
        "id#1",
        "user$name",
        "foo!bar",
        "a+b",
        "a=b",
    ] {
        assert_eq!(
            DisplayValue::server_id(id),
            Err(DisplayValueError::ServerId(id.to_string()))
        );
    }
}
