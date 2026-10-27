//! R2a (design `rate-alerts.md` R2, R2a): the customer is told once that a rate alert was turned off because the
//! branch now quotes the currency per a different unit (`UNIT_CHANGED`). Transactional. Params `currency` and `side`
//! only (customer words, as `rate_alert_crossed`). The `zh` text needs a human check (L7, #1605).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "rate_alert_unit_changed";

pub struct Template;

pub type RateAlertUnitChangedTemplate = Template;

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
                title: "แจ้งเตือนอัตราแลกเปลี่ยนถูกปิด".to_string(),
                body: format!(
                    "การแจ้งเตือนอัตราแลกเปลี่ยนสำหรับ {currency} ถูกปิดเนื่องจากสาขาเปลี่ยนหน่วยอัตราแลกเปลี่ยน กรุณาตั้งค่าใหม่อีกครั้ง"
                ),
            },
            Locale::En => Rendered {
                title: "Rate Alert Disabled".to_string(),
                body: format!(
                    "Your rate alert for {currency} was turned off because the branch changed the rate unit. Please set it again."
                ),
            },
            Locale::Zh => Rendered {
                title: "汇率提醒已关闭".to_string(),
                body: format!(
                    "由于网点更改了报价单位，您的 {currency} 汇率提醒已关闭，请重新设置。"
                ),
            },
        }
    }
}
