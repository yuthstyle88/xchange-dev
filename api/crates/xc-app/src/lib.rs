//! `xc-app` — the customer app's server half (design `docs/design/blueprint/customer-app.md`, #558).
//!
//! APP-4 (#994) creates the crate with the planner (P5–P7, P13): `PlannerDraft`, `DraftOwner`,
//! `PlannerService`, `PlannerPreview`, the anonymous `AppSession` and the organisation's
//! `AppSettings` (owner C4). APP-6 adds the assistant beside these
//! (`src/{domain,application}/assistant/**`, `infrastructure/assistant_repository.rs`).

pub mod application;
pub mod domain;
pub mod infrastructure;
