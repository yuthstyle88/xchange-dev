//! Notification template: rate_sharp_move (SM-1 #1490, rates.md §12.8, §12.10).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "rate_sharp_move";

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
        let currency = params.get("currency").map(|s| s.as_str()).unwrap_or("");
        let side = params.get("side").map(|s| s.as_str()).unwrap_or("");

        match locale {
            Locale::Th => Rendered {
                title: "อัตราแลกเปลี่ยนเปลี่ยนแปลงอย่างมีนัยสำคัญ".to_string(),
                body: format!(
                    "อัตราแลกเปลี่ยน {currency} ({side}) ที่สาขา {branch} มีการเปลี่ยนแปลงเกินเกณฑ์ที่กำหนด โปรดตรวจสอบประกาศอัตราแลกเปลี่ยน"
                ),
            },
            Locale::En => Rendered {
                title: "Sharp rate movement".to_string(),
                body: format!(
                    "Rate for {currency} ({side}) at branch {branch} moved beyond the threshold. Please check the rate publication."
                ),
            },
            Locale::Zh => Rendered {
                title: "汇率大幅变动".to_string(),
                body: format!(
                    "{branch} 分行的 {currency} ({side}) 汇率变动幅度超过设定阈值，请查看最新汇率发布。"
                ),
            },
        }
    }
}
