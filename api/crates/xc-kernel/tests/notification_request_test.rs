//! APP-2 (#558): `NotificationRequest`, the producers' input to the `Notifications` port, design
//! `customer-app.md` P8, P10 (params hold references and display values, never PII). Written before
//! the implementation by the ticket author (claude/5edf7bb5). Do not edit, delete or weaken; add tests
//! in a file of your own.

use std::collections::BTreeMap;

use xc_kernel::notifications::{NotificationObject, NotificationRequest, NotificationRequestError};

const USER: &str = "6f1c1d2e-0000-4000-8000-000000000001";

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn a_request_with_references_and_display_values_is_built() {
    let object = NotificationObject {
        object_type: "queue_ticket".into(),
        object_id: "T-1".into(),
    };
    let request = NotificationRequest::new(
        USER,
        "queue.called:T-1:1",
        "queue_called",
        Some(object.clone()),
        params(&[
            ("displayNumber", "W001"),
            ("counter", "3"),
            ("branch", "Siam"),
        ]),
    )
    .expect("references and display values are allowed");
    assert_eq!(request.recipient_user_id(), USER);
    assert_eq!(request.event_id(), "queue.called:T-1:1");
    assert_eq!(request.template_key(), "queue_called");
    assert_eq!(request.object(), Some(&object));
    assert_eq!(
        request.params().get("displayNumber").map(String::as_str),
        Some("W001")
    );
}

#[test]
fn a_param_holding_a_phone_is_refused() {
    let err = NotificationRequest::new(
        USER,
        "e:1",
        "queue_called",
        None,
        params(&[("displayNumber", "W001"), ("phone", "0812345678")]),
    )
    .expect_err("a phone in params must be refused");
    assert_eq!(err, NotificationRequestError::PersonalData("phone".into()));
}

#[test]
fn a_param_holding_an_email_is_refused() {
    let err = NotificationRequest::new(USER, "e:1", "t", None, params(&[("to", "a@b.co")]))
        .expect_err("an e-mail in params must be refused");
    assert_eq!(err, NotificationRequestError::PersonalData("to".into()));
}

#[test]
fn empty_identifiers_are_refused() {
    assert_eq!(
        NotificationRequest::new("", "e:1", "t", None, BTreeMap::new()).unwrap_err(),
        NotificationRequestError::Empty("recipient_user_id")
    );
    assert_eq!(
        NotificationRequest::new(USER, "", "t", None, BTreeMap::new()).unwrap_err(),
        NotificationRequestError::Empty("event_id")
    );
    assert_eq!(
        NotificationRequest::new(USER, "e:1", "", None, BTreeMap::new()).unwrap_err(),
        NotificationRequestError::Empty("template_key")
    );
}
