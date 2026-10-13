//! Notification template: queue_called (APP-10 #1001, customer-app.md P8–P10, §4.1, §5).

use std::collections::BTreeMap;

use crate::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const KEY: &str = "queue_called";

pub struct Template;

pub type QueueCalledTemplate = Template;

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
        let display_number = params.get("displayNumber").map(|s| s.as_str()).unwrap_or("");
        let branch_name = params.get("branchName").map(|s| s.as_str()).unwrap_or("");
        match locale {
            Locale::Th => Rendered {
                title: "เรียกคิวแล้ว".to_string(),
                body: format!("หมายเลขคิว {display_number} ที่สาขา {branch_name} ถึงคิวของคุณแล้ว"),
            },
            Locale::En => Rendered {
                title: "Queue ticket called".to_string(),
                body: format!("Queue ticket {display_number} is now called at {branch_name}."),
            },
            Locale::Zh => Rendered {
                title: "叫号提醒".to_string(),
                body: format!("{branch_name} 的排队号码 {display_number} 已叫号。"),
            },
        }
    }
}
