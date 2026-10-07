//! Device DTOs: job, event, and overview JSON serializers for POST /api/devices.

use chrono::SecondsFormat;
use serde_json::{Value, json};

use crate::infrastructure::devices::device_event::DeviceEvent;
use crate::infrastructure::devices::device_job::DeviceJob;
use crate::infrastructure::devices::device_service::DeviceOverview;
use crate::infrastructure::devices::printer_models::DrawerSettings;

/// A device job object as v2 answered it (minus request_hash).
pub fn job_json(job: &DeviceJob) -> Value {
    json!({
        "id": job.id,
        "request_id": job.request_id,
        "branch_id": job.branch_id,
        "ref": job.reference,
        "kind": job.kind.as_str(),
        "status": job.status.as_str(),
        "actor": job.actor,
        "created": job.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "updated": job.updated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

/// A device event line in the snapshot.
pub fn event_json(event: &DeviceEvent) -> Value {
    json!({
        "kind": event.kind,
        "ref": event.reference,
        "at": event.at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "actor": event.actor,
        "detail": event.detail,
        "branch_id": event.branch_id,
    })
}

/// The overview (snapshot) of device jobs, events, and printer status.
pub fn overview_json(overview: &DeviceOverview, drawer: &DrawerSettings, model: Value) -> Value {
    json!({
        "jobs": overview.jobs.iter().map(job_json).collect::<Vec<_>>(),
        "events": overview.events.iter().map(event_json).collect::<Vec<_>>(),
        "printerOnline": overview.printer_online,
        "printerModel": drawer.model,
        "hasDrawer": drawer.has_drawer,
        "model": model,
    })
}
