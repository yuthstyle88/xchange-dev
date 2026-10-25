//! Notification template: shift_close_certified (#5096).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "shift_close_certified";

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

    fn render(&self, locale: Locale, params: &BTreeMap<String, String>) -> Rendered {
        let branch = params.get("branch").map(|s| s.as_str()).unwrap_or("");
        let shift = params.get("shift").map(|s| s.as_str()).unwrap_or("");
        let by = params.get("by").map(|s| s.as_str()).unwrap_or("");

        match locale {
            Locale::Th => Rendered {
                title: "รับรองการปิดกะเรียบร้อยแล้ว".to_string(),
                body: format!("การนับเงินปิดกะ {shift} ที่สาขา {branch} ได้รับการรับรองโดย {by}"),
            },
            Locale::En => Rendered {
                title: "Shift close certified".to_string(),
                body: format!(
                    "Shift close for shift {shift} at branch {branch} was certified by {by}."
                ),
            },
            Locale::Zh => Rendered {
                title: "交班清点已确认".to_string(),
                body: format!("{branch} 分行班次 {shift} 的交班清点已由 {by} 确认。"),
            },
        }
    }
}
