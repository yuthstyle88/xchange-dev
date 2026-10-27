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
        let _ = (locale, params);
        todo!("R2a: TH/EN/ZH text naming the currency")
    }
}
