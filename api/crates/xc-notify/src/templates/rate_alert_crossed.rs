//! Rate alert crossed notification template (TH/EN/ZH).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "rate_alert_crossed";

pub struct Template;

pub type RateAlertCrossedTemplate = Template;

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
        let currency = params.get("currency").map(|s| s.as_str()).unwrap_or("");
        match locale {
            Locale::Th => Rendered {
                title: "แจ้งเตือนอัตราแลกเปลี่ยน".to_string(),
                body: format!("อัตราแลกเปลี่ยนสำหรับ {currency} ถึงระดับเป้าหมายแล้ว"),
            },
            Locale::En => Rendered {
                title: "Rate Alert".to_string(),
                body: format!("Target rate reached for {currency}."),
            },
            Locale::Zh => Rendered {
                title: "汇率提醒".to_string(),
                body: format!("{currency} 汇率已达到设定的目标"),
            },
        }
    }
}
