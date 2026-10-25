//! Notification template: close_count_waiting (#5096, #5113).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "close_count_waiting";

pub struct Template;

pub(crate) fn close_amount_suffix(locale: Locale, params: &BTreeMap<String, String>) -> String {
    let Some(c) = params.get("counted_thb").map(|s| s.as_str()) else {
        return String::new();
    };
    match (locale, params.get("expected_thb").map(|s| s.as_str())) {
        (Locale::Th, Some(e)) => format!(" (นับได้ {c} บาท, ควรมี {e} บาท)"),
        (Locale::Th, None) => format!(" (นับได้ {c} บาท)"),
        (Locale::En, Some(e)) => format!(" Counted THB {c}, expected THB {e}."),
        (Locale::En, None) => format!(" Counted THB {c}."),
        (Locale::Zh, Some(e)) => format!("清点 {c} 泰铢，应有 {e} 泰铢。"),
        (Locale::Zh, None) => format!("清点 {c} 泰铢。"),
    }
}

impl NotificationTemplate for Template {
    fn key(&self) -> &'static str {
        KEY
    }

    fn version(&self) -> i32 {
        2
    }

    fn kind(&self) -> TemplateKind {
        TemplateKind::Transactional
    }

    fn render(&self, locale: Locale, params: &BTreeMap<String, String>) -> Rendered {
        let branch = params.get("branch").map(|s| s.as_str()).unwrap_or("");
        let shift = params.get("shift").map(|s| s.as_str()).unwrap_or("");
        let by = params.get("by").map(|s| s.as_str()).unwrap_or("");
        let suffix = close_amount_suffix(locale, params);

        match locale {
            Locale::Th => Rendered {
                title: "การนับเงินปิดกะรอการรับรอง".to_string(),
                body: format!(
                    "มีการส่งยอดนับเงินปิดกะ {shift} ที่สาขา {branch} โดย {by} รอการรับรอง{suffix}"
                ),
            },
            Locale::En => Rendered {
                title: "Close count waiting for certification".to_string(),
                body: format!(
                    "Close count for shift {shift} at branch {branch} submitted by {by} is waiting for certification.{suffix}"
                ),
            },
            Locale::Zh => Rendered {
                title: "交班清点待确认".to_string(),
                body: format!(
                    "{branch} 分行班次 {shift} 的交班清点（由 {by} 提交）正在等待确认。{suffix}"
                ),
            },
        }
    }
}
