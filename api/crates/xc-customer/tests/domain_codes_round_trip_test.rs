use std::str::FromStr;

use xc_customer::domain::consent::{Channel, Purpose};
use xc_customer::domain::retention::DataType;

fn all_purposes() -> &'static [Purpose] {
    fn check_exhaustive(p: Purpose) {
        match p {
            Purpose::Terms => {}
            Purpose::Marketing => {}
        }
    }
    let _ = check_exhaustive;

    &[Purpose::Terms, Purpose::Marketing]
}

fn expected_purpose_code(p: Purpose) -> &'static str {
    match p {
        Purpose::Terms => "TERMS",
        Purpose::Marketing => "MARKETING",
    }
}

fn all_channels() -> &'static [Channel] {
    fn check_exhaustive(c: Channel) {
        match c {
            Channel::Sms => {}
            Channel::Email => {}
            Channel::Push => {}
        }
    }
    let _ = check_exhaustive;

    &[Channel::Sms, Channel::Email, Channel::Push]
}

fn expected_channel_code(c: Channel) -> &'static str {
    match c {
        Channel::Sms => "SMS",
        Channel::Email => "EMAIL",
        Channel::Push => "PUSH",
    }
}

fn all_data_types() -> &'static [DataType] {
    fn check_exhaustive(d: DataType) {
        match d {
            DataType::IdentityValue => {}
            DataType::Contact => {}
            DataType::ClosedCustomer => {}
            DataType::RateAlert => {}
            DataType::AssistantConversation => {}
            DataType::PlannerDraft => {}
            DataType::VerificationPassport => {}
            DataType::VerificationNationalId => {}
        }
    }
    let _ = check_exhaustive;

    &[
        DataType::IdentityValue,
        DataType::Contact,
        DataType::ClosedCustomer,
        DataType::RateAlert,
        DataType::AssistantConversation,
        DataType::PlannerDraft,
        DataType::VerificationPassport,
        DataType::VerificationNationalId,
    ]
}

fn expected_data_type_code(d: DataType) -> &'static str {
    match d {
        DataType::IdentityValue => "IDENTITY_VALUE",
        DataType::Contact => "CONTACT",
        DataType::ClosedCustomer => "CLOSED_CUSTOMER",
        DataType::RateAlert => "RATE_ALERT",
        DataType::AssistantConversation => "ASSISTANT_CONVERSATION",
        DataType::PlannerDraft => "PLANNER_DRAFT",
        DataType::VerificationPassport => "VERIFICATION_PASSPORT",
        DataType::VerificationNationalId => "VERIFICATION_NATIONAL_ID",
    }
}

#[test]
fn row1_purpose_from_str() {
    for &variant in all_purposes() {
        let code = expected_purpose_code(variant);
        assert_eq!(Purpose::from_str(code), Ok(variant));
        assert_eq!(code.parse::<Purpose>(), Ok(variant));
        assert_eq!(variant.as_str(), code);
        assert_eq!(variant.to_string(), code);
    }
}

#[test]
fn row2_channel_from_str() {
    for &variant in all_channels() {
        let code = expected_channel_code(variant);
        assert_eq!(Channel::from_str(code), Ok(variant));
        assert_eq!(code.parse::<Channel>(), Ok(variant));
        assert_eq!(variant.as_str(), code);
        assert_eq!(variant.to_string(), code);
    }
}

#[test]
fn row3_data_type_from_str() {
    for &variant in all_data_types() {
        let code = expected_data_type_code(variant);
        assert_eq!(DataType::from_str(code), Ok(variant));
        assert_eq!(code.parse::<DataType>(), Ok(variant));
        assert_eq!(variant.as_str(), code);
        assert_eq!(variant.to_string(), code);
    }
}

#[test]
fn row4_serde_json_round_trip() {
    for &variant in all_purposes() {
        let code = expected_purpose_code(variant);
        let expected_json = format!("\"{code}\"");
        let serialized = serde_json::to_string(&variant).expect("serialize purpose");
        assert_eq!(serialized, expected_json);
        let deserialized: Purpose =
            serde_json::from_str(&expected_json).expect("deserialize purpose");
        assert_eq!(deserialized, variant);
    }

    for &variant in all_channels() {
        let code = expected_channel_code(variant);
        let expected_json = format!("\"{code}\"");
        let serialized = serde_json::to_string(&variant).expect("serialize channel");
        assert_eq!(serialized, expected_json);
        let deserialized: Channel =
            serde_json::from_str(&expected_json).expect("deserialize channel");
        assert_eq!(deserialized, variant);
    }

    for &variant in all_data_types() {
        let code = expected_data_type_code(variant);
        let expected_json = format!("\"{code}\"");
        let serialized = serde_json::to_string(&variant).expect("serialize data_type");
        assert_eq!(serialized, expected_json);
        let deserialized: DataType =
            serde_json::from_str(&expected_json).expect("deserialize data_type");
        assert_eq!(deserialized, variant);
    }
}

#[test]
fn row5_purpose_and_channel_invalid_and_case_rejected() {
    let invalid_purposes = [
        "terms",
        "Terms",
        "marketing",
        "Marketing",
        "",
        "UNKNOWN",
        " ",
    ];
    for &s in &invalid_purposes {
        assert!(
            Purpose::from_str(s).is_err(),
            "Purpose::from_str({s:?}) should be Err"
        );
        assert!(
            s.parse::<Purpose>().is_err(),
            "{s:?}.parse::<Purpose>() should be Err"
        );
        assert!(
            serde_json::from_str::<Purpose>(&format!("\"{s}\"")).is_err(),
            "serde_json::from_str::<Purpose>(\"{s}\") should be Err"
        );
    }

    let invalid_channels = [
        "sms", "Sms", "email", "Email", "push", "Push", "", "UNKNOWN", " ",
    ];
    for &s in &invalid_channels {
        assert!(
            Channel::from_str(s).is_err(),
            "Channel::from_str({s:?}) should be Err"
        );
        assert!(
            s.parse::<Channel>().is_err(),
            "{s:?}.parse::<Channel>() should be Err"
        );
        assert!(
            serde_json::from_str::<Channel>(&format!("\"{s}\"")).is_err(),
            "serde_json::from_str::<Channel>(\"{s}\") should be Err"
        );
    }
}

#[test]
fn row6_data_type_invalid_and_case_rejected() {
    let invalid_data_types = [
        "identity_value",
        "Identity_Value",
        "contact",
        "Contact",
        "closed_customer",
        "rate_alert",
        "assistant_conversation",
        "planner_draft",
        "verification_passport",
        "verification_national_id",
        "",
        "UNKNOWN",
        " ",
    ];
    for &s in &invalid_data_types {
        assert!(
            DataType::from_str(s).is_err(),
            "DataType::from_str({s:?}) should be Err"
        );
        assert!(
            s.parse::<DataType>().is_err(),
            "{s:?}.parse::<DataType>() should be Err"
        );
        assert!(
            serde_json::from_str::<DataType>(&format!("\"{s}\"")).is_err(),
            "serde_json::from_str::<DataType>(\"{s}\") should be Err"
        );
    }
}
