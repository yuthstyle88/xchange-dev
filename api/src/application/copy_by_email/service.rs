//! `CopyByEmailService` (design `kiosk-shared-phone.md` §4.2 D21, D22; §4.4 B19–B25): the core of
//! the e-mail copy. Every rule is here; a `CopySubject` only loads its row.
//!
//! `[required]` order of `send`'s checks (each failure sends nothing and stores nothing):
//! 1. the subject: no `CopySubject` for `kind`, or `load_for(member, id, SHA-256(qr))` is `None`
//!    ⇒ `not_found()` (`404 notFound`);
//! 2. the member's verified e-mail (`MemberEmails`): none ⇒ `no_verified_email()`
//!    (`409 noVerifiedEmail`);
//! 3. the sender: `!is_configured()` ⇒ `email_unavailable()` (`503 emailUnavailable`); a send
//!    failure passes through as the sender reports it.
//!
//! The message: `to` = that verified address; `idempotency_key` =
//! `copy:<subject>:<id>:<lowercase hex of SHA-256(qr)>`; text and HTML in the member's `lang`
//! (th/en/zh) with the details rows, the branch, and the link `CopyLink::url(origin, sign(…, now +
//! 24 h))`; the HTML also carries the QR as `<img src="data:image/png;base64,…">` (`qr_png`).
//! Never a passport, a phone or a customer number.

use std::sync::Arc;

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use xc_customer::domain::email_sender::{
    EMAIL_UNAVAILABLE, EmailMessage, EmailOutcome, EmailSender,
};

use super::link::{CopyLink, LINK_LIFETIME_HOURS};
use super::member_email::MemberEmails;
use super::subject::{CopyDetails, CopySubject, CopySubjectKind};

/// What `copyByEmail` answers: `{sent: true}`, or `{sent: true, again: true}` when Notify
/// recognised the idempotency key (B20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub again: bool,
}

/// What the link page shows: the details and the live status, plus the QR from the link itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyView {
    pub details: CopyDetails,
    pub qr: String,
    pub expires_at: DateTime<Utc>,
}

/// `404 notFound`: another member's, unknown, or a reissued-away QR — one answer.
pub fn not_found() -> AppError {
    AppError::NotFound {
        entity: "copySubject",
        id: String::new(),
    }
}

/// `409 noVerifiedEmail`.
pub fn no_verified_email() -> AppError {
    AppError::Conflict("noVerifiedEmail".to_owned())
}

/// `503 emailUnavailable`.
pub fn email_unavailable() -> AppError {
    AppError::Domain(EMAIL_UNAVAILABLE.to_owned())
}

/// `410 linkExpired`: every link failure, one answer.
pub fn link_expired() -> AppError {
    AppError::Validation("linkExpired".to_owned())
}

#[derive(Clone)]
pub struct CopyByEmailService {
    subjects: Vec<Arc<dyn CopySubject>>,
    members: Arc<dyn MemberEmails>,
    sender: Arc<dyn EmailSender>,
    link: CopyLink,
    origin: String,
}

impl CopyByEmailService {
    /// `origin` = the site the link opens on (`https://host`, no trailing slash needed).
    pub fn new(
        subjects: Vec<Arc<dyn CopySubject>>,
        members: Arc<dyn MemberEmails>,
        sender: Arc<dyn EmailSender>,
        link: CopyLink,
        origin: String,
    ) -> Self {
        Self {
            subjects,
            members,
            sender,
            link,
            origin,
        }
    }

    /// `copyByEmail` (B19–B23): see the module header for the order and the message.
    pub async fn send(
        &self,
        member: &str,
        kind: CopySubjectKind,
        id: &str,
        qr: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Sent> {
        let subject = self
            .subjects
            .iter()
            .find(|s| s.kind() == kind)
            .ok_or_else(not_found)?;
        let qr_hash = sha256(qr);
        let details = subject
            .load_for(member, id, &qr_hash)
            .await?
            .ok_or_else(not_found)?;
        let email = self
            .members
            .verified_email(member)
            .await?
            .ok_or_else(no_verified_email)?;
        if !self.sender.is_configured() {
            return Err(email_unavailable());
        }

        let signed = self
            .link
            .sign(kind, id, qr, now + Duration::hours(LINK_LIFETIME_HOURS));
        let link_url = CopyLink::url(&self.origin, &signed);
        let message = build_message(
            &email.address,
            &email.lang,
            &details,
            qr,
            &qr_hash,
            &link_url,
        );
        let outcome = self.sender.send(&message).await?;
        Ok(Sent {
            again: matches!(outcome, EmailOutcome::AlreadySent),
        })
    }

    /// `copyLinkView` (B24, B25): `CopyLink::verify`, then the subject's `load_by_link(id,
    /// SHA-256(qr))`. Any failure, a subject with no implementation included ⇒ `link_expired()`.
    pub async fn view_link(&self, link: &str, now: DateTime<Utc>) -> AppResult<CopyView> {
        let parts = self.link.verify(link, now).map_err(|_| link_expired())?;
        let subject = self
            .subjects
            .iter()
            .find(|s| s.kind() == parts.kind)
            .ok_or_else(link_expired)?;
        let qr_hash = sha256(&parts.qr);
        let details = subject
            .load_by_link(&parts.id, &qr_hash)
            .await?
            .ok_or_else(link_expired)?;
        Ok(CopyView {
            details,
            qr: parts.qr,
            expires_at: parts.expires_at,
        })
    }
}

/// The QR of `data` as a PNG (the e-mail's inline picture): `qrcode` for the modules, `png` for the
/// file, black on white with a 4-module quiet zone. The same data always gives the same bytes.
pub fn qr_png(data: &str) -> Vec<u8> {
    const BORDER: usize = 4;
    const SCALE: usize = 8;

    let code = qrcode::QrCode::new(data.as_bytes()).expect("encode QR");
    let width = code.width();
    let side = (width + BORDER * 2) * SCALE;
    let mut pixels = vec![255u8; side * side];
    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == qrcode::Color::Dark {
                let px0 = (x + BORDER) * SCALE;
                let py0 = (y + BORDER) * SCALE;
                for dy in 0..SCALE {
                    let row = (py0 + dy) * side + px0;
                    pixels[row..row + SCALE].fill(0);
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side as u32, side as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(&pixels).expect("PNG data");
    drop(writer);
    out
}

fn sha256(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_message(
    to: &str,
    lang: &str,
    details: &CopyDetails,
    qr: &str,
    qr_hash: &[u8; 32],
    link_url: &str,
) -> EmailMessage {
    let subject = match lang {
        "th" => format!("สำเนาข้อมูล — {}", details.branch_name),
        "zh" => format!("信息副本 — {}", details.branch_name),
        _ => format!("Your copy — {}", details.branch_name),
    };
    let mut text = format!(
        "{}: {}\n{}: {}\n",
        label(lang, "branch"),
        details.branch_name,
        label(lang, "status"),
        details.status,
    );
    let mut rows_html = format!(
        "<tr><th>{}</th><td>{}</td></tr><tr><th>{}</th><td>{}</td></tr>",
        escape_html(label(lang, "branch")),
        escape_html(&details.branch_name),
        escape_html(label(lang, "status")),
        escape_html(&details.status),
    );
    for (field, value) in &details.rows {
        let lbl = label(lang, field);
        text.push_str(&format!("{lbl}: {value}\n"));
        rows_html.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            escape_html(lbl),
            escape_html(value),
        ));
    }
    let link_lbl = label(lang, "link");
    text.push_str(&format!("{link_lbl}: {link_url}\n"));
    let b64 = base64::engine::general_purpose::STANDARD.encode(qr_png(qr));
    let html = format!(
        "<div><table>{rows_html}</table><p><img src=\"data:image/png;base64,{b64}\" alt=\"QR\"></p><p><a href=\"{link_url}\">{}</a></p></div>",
        escape_html(link_lbl),
    );
    EmailMessage {
        to: to.to_owned(),
        subject,
        text,
        html,
        idempotency_key: format!(
            "copy:{}:{}:{}",
            details.kind.as_str(),
            details.id,
            hex_lower(qr_hash),
        ),
    }
}

fn label<'a>(lang: &str, field: &'a str) -> &'a str {
    match (lang, field) {
        ("th", "branch") => "สาขา",
        ("zh", "branch") => "分行",
        (_, "branch") => "Branch",
        ("th", "status") => "สถานะ",
        ("zh", "status") => "状态",
        (_, "status") => "Status",
        ("th", "bookingNo") => "เลขที่การจอง",
        ("zh", "bookingNo") => "预约号",
        (_, "bookingNo") => "Booking No.",
        ("th", "date") => "วันที่",
        ("zh", "date") => "日期",
        (_, "date") => "Date",
        ("th", "slot") => "ช่วงเวลา",
        ("zh", "slot") => "时段",
        (_, "slot") => "Time slot",
        ("th", "currency") => "สกุลเงิน",
        ("zh", "currency") => "币种",
        (_, "currency") => "Currency",
        ("th", "amount") => "จำนวน",
        ("zh", "amount") => "金额",
        (_, "amount") => "Amount",
        ("th", "rate") => "อัตราแลกเปลี่ยน",
        ("zh", "rate") => "汇率",
        (_, "rate") => "Rate",
        ("th", "expiresAt") => "หมดอายุ",
        ("zh", "expiresAt") => "有效期至",
        (_, "expiresAt") => "Expires at",
        ("th", "link") => "ลิงก์ (ใช้ได้ 24 ชม.)",
        ("zh", "link") => "链接（24小时内有效）",
        (_, "link") => "Link (valid 24 hours)",
        (_, other) => other,
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
