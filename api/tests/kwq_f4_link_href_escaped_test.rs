//! KWQ-F4 — `build_message` escapes `link_url` inside `href`.

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use pos_error::AppResult;
use xc_customer::domain::email_sender::{EmailMessage, EmailOutcome, EmailSender};
use xc_customer::domain::sms_sender::BoxFuture;
use xchange::application::copy_by_email::link::CopyLink;
use xchange::application::copy_by_email::member_email::{MemberEmails, VerifiedEmail};
use xchange::application::copy_by_email::service::CopyByEmailService;
use xchange::application::copy_by_email::subject::{CopyDetails, CopySubject, CopySubjectKind};

struct FakeSubject;

impl CopySubject for FakeSubject {
    fn kind(&self) -> CopySubjectKind {
        CopySubjectKind::Booking
    }

    fn load_for<'a>(
        &'a self,
        _member: &'a str,
        id: &'a str,
        _qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move {
            Ok(Some(CopyDetails {
                kind: CopySubjectKind::Booking,
                id: id.to_owned(),
                branch_name: "Silom".to_owned(),
                rows: vec![],
                status: "QUOTED".to_owned(),
            }))
        })
    }

    fn load_by_link<'a>(
        &'a self,
        _id: &'a str,
        _qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move { Ok(None) })
    }
}

struct FakeMembers;

impl MemberEmails for FakeMembers {
    fn verified_email<'a>(
        &'a self,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<VerifiedEmail>>> {
        Box::pin(async move {
            Ok(Some(VerifiedEmail {
                address: "alice@example.com".to_owned(),
                lang: "en".to_owned(),
            }))
        })
    }
}

struct FakeSender(Mutex<Vec<EmailMessage>>);

impl EmailSender for FakeSender {
    fn is_configured(&self) -> bool {
        true
    }

    fn send<'a>(&'a self, message: &'a EmailMessage) -> BoxFuture<'a, AppResult<EmailOutcome>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(message.clone());
            Ok(EmailOutcome::Sent)
        })
    }
}

#[tokio::test]
async fn link_href_escapes_quotes_and_ampersands_while_plain_text_keeps_raw_url() {
    let origin = "https://xc-v3.example/\"&<x>";
    let sender = Arc::new(FakeSender(Mutex::new(Vec::new())));
    let service = CopyByEmailService::new(
        vec![Arc::new(FakeSubject)],
        Arc::new(FakeMembers),
        sender.clone(),
        CopyLink::new([9; 32]),
        origin.to_owned(),
    );
    let now = Utc
        .with_ymd_and_hms(2026, 10, 22, 1, 0, 0)
        .single()
        .unwrap();
    service
        .send(
            "CUST-ALICE",
            CopySubjectKind::Booking,
            "BK-1",
            "bk-qr-1",
            now,
        )
        .await
        .expect("sent");

    let sent = sender.0.lock().unwrap();
    let msg = &sent[0];
    assert!(msg.text.contains("https://xc-v3.example/\"&<x>/c#"));
    assert!(
        msg.html
            .contains("href=\"https://xc-v3.example/&quot;&amp;&lt;x&gt;/c#"),
        "html: {}",
        msg.html
    );
    let href_start = msg.html.find("href=\"").expect("href") + "href=\"".len();
    let href_val = &msg.html[href_start..href_start + msg.html[href_start..].find('"').unwrap()];
    assert!(!href_val.contains('"'));
    assert!(href_val.contains("&quot;") && href_val.contains("&amp;"));
}
