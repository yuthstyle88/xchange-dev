//! DTO types for `my_notifications` view (§4.4, §7.7).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_notifications.ts")]
pub struct MyNotificationObject {
    pub object_type: String,
    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_notifications.ts")]
pub struct MyNotificationItem {
    pub id: String,
    #[ts(type = "number")]
    pub seq: i64,
    pub locale: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub object: Option<MyNotificationObject>,
    pub read: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_notifications.ts")]
pub struct MyNotificationsData {
    pub items: Vec<MyNotificationItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "my_notifications.ts")]
pub struct MyNotificationsFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unread: Option<bool>,
}

impl ViewFilter for MyNotificationsFilter {}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyNotificationObject::export_all(&cfg)?;
    MyNotificationItem::export_all(&cfg)?;
    MyNotificationsData::export_all(&cfg)?;
    MyNotificationsFilter::export_all(&cfg)?;
    Ok(())
}
