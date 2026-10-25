//! Notification template: close_count_waiting (#5096).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "close_count_waiting";

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
                title: "การนับเงินปิดกะรอการรับรอง".to_string(),
                body: format!("มีการส่งยอดนับเงินปิดกะ {shift} ที่สาขา {branch} โดย {by} รอการรับรอง"),
            },
            Locale::En => Rendered {
                title: "Close count waiting for certification".to_string(),
                body: format!(
                    "Close count for shift {shift} at branch {branch} submitted by {by} is waiting for certification."
                ),
            },
            Locale::Zh => Rendered {
                title: "交班清点待确认".to_string(),
                body: format!("{branch} 分行班次 {shift} 的交班清点（由 {by} 提交）正在等待确认。"),
            },
        }
    }
}
