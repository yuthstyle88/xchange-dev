//! Notification template: count_verified (#5643).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "count_verified";

pub struct Template;

impl NotificationTemplate for Template {
    fn key(&self) -> &'static str {
        KEY
    }

    fn version(&self) -> i32 {
        1
    }

    fn kind(&self) -> TemplateKind {
        TemplateKind::Transactional
    }

    fn render(&self, _locale: Locale, params: &BTreeMap<String, String>) -> Rendered {
        let kind_name = match params.get("kind").map(|s| s.as_str()).unwrap_or("") {
            "OPEN" => "นับเปิดกะ",
            "CLOSE" => "นับปิดกะ",
            "SAFE_OPEN" => "นับเปิดตู้เซฟ",
            "SAFE" => "นับตู้เซฟ",
            other => other,
        };
        let branch = params.get("branch").map(|s| s.as_str()).unwrap_or("");
        let counted_on = params.get("counted_on").map(|s| s.as_str()).unwrap_or("");
        let counted_time = params.get("counted_time").map(|s| s.as_str()).unwrap_or("");

        Rendered {
            title: "ยอดนับเงินได้รับการรับรองแล้ว".to_string(),
            body: format!(
                "ยอด{kind_name}ของคุณ ({counted_on} {counted_time}) ที่สาขา {branch} ได้รับการรับรองแล้ว"
            ),
        }
    }
}
