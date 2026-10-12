//! EX-3 #1071: A01's answer, `GET /api/executive/overview` (design `executive-finance.md` §8.1), camelCase. Money is
//! a decimal string with two places.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::application::executive::drilldown::Drilldown;
use crate::application::executive::metrics::Metric;
use crate::application::executive::overview::{BranchRow, Overview};
use crate::application::executive::ports::{Alert, ExecutiveFilter};

/// An organisation view's answer (D-5): the versions of the branches in scope; `data` only when one moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct OrgViewAnswer<T: TS> {
    pub changed: bool,
    #[ts(type = "Record<string, number>")]
    pub versions: BTreeMap<String, i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub data: Option<T>,
}

/// A01's query keys (design §4.1, §8.1). `branches` is comma-separated; `versions` is a JSON map of branch to
/// version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct ExecutiveOverviewFilter {
    #[ts(optional)]
    pub from: Option<String>,
    #[ts(optional)]
    pub to: Option<String>,
    #[ts(optional)]
    pub branches: Option<String>,
    #[ts(optional, type = "\"WalkIn\" | \"Booking\" | \"Kiosk\" | \"Referred\"")]
    pub channel: Option<String>,
    #[ts(optional)]
    pub currency: Option<String>,
    #[ts(optional)]
    pub versions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct ExecutiveOverviewData {
    pub period: OverviewPeriod,
    pub kpis: Vec<MetricDto>,
    pub branches: Vec<BranchRowDto>,
    pub alerts: Vec<AlertDto>,
    pub definitions: Vec<DefinitionDto>,
    pub buy_reversed_thb: String,
    pub sell_reversed_thb: String,
}

/// The business dates asked for; `null` means today in each branch's timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct OverviewPeriod {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// A figure: a value, or why there is none (D-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct MetricDto {
    pub key: String,
    pub value: Option<String>,
    #[ts(type = "\"awaitsLedger\" | \"awaitsBookingDesign\" | \"awaitsBroker\" | \"awaitsCampaigns\" | null")]
    pub unavailable: Option<String>,
    pub drilldown: Option<DrilldownDto>,
}

/// A paged view, the filter it is sent, and A01's keys it does not take yet (design §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct DrilldownDto {
    pub view: String,
    #[ts(type = "Record<string, string>")]
    pub filter: BTreeMap<String, String>,
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct ShiftsDto {
    #[ts(type = "number")]
    pub open: i64,
    #[ts(type = "number")]
    pub closed_verified: i64,
    #[ts(type = "number")]
    pub closed_unverified: i64,
}

/// A row of the branch table (design §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct BranchRowDto {
    pub branch_id: String,
    pub name: String,
    pub volume_thb: String,
    pub available_cash: MetricDto,
    pub in_transit: MetricDto,
    pub shifts: ShiftsDto,
    pub as_of: Option<String>,
    #[ts(type = "number")]
    pub pending_events: i64,
    pub drilldown: DrilldownDto,
}

/// A "ต้องดำเนินการ" item (design §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct AlertDto {
    #[ts(type = "\"ratePending\" | \"staleData\" | \"countDifference\"")]
    pub kind: String,
    pub branch_id: String,
    pub since: Option<String>,
    pub detail: String,
    pub drilldown: Option<DrilldownDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "executive_overview.ts")]
pub struct DefinitionDto {
    pub key: String,
    pub th: String,
    pub en: String,
}

fn time(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl DrilldownDto {
    fn of(d: &Drilldown) -> Self {
        Self {
            view: d.view.to_owned(),
            filter: d
                .filter
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            dropped: d.dropped.iter().map(|k| (*k).to_owned()).collect(),
        }
    }
}

impl MetricDto {
    fn of(m: &Metric) -> Self {
        Self {
            key: m.key.to_owned(),
            value: m.value.clone(),
            unavailable: m.unavailable.map(|u| u.as_str().to_owned()),
            drilldown: m.drilldown.as_ref().map(DrilldownDto::of),
        }
    }
}

impl BranchRowDto {
    fn of(r: &BranchRow) -> Self {
        Self {
            branch_id: r.head.id.as_str().to_owned(),
            name: r.head.name.clone(),
            volume_thb: crate::application::executive::metrics::thb(r.volume_thb),
            available_cash: MetricDto::of(&r.available_cash),
            in_transit: MetricDto::of(&r.in_transit),
            shifts: ShiftsDto {
                open: r.shifts.open,
                closed_verified: r.shifts.closed_verified,
                closed_unverified: r.shifts.closed_unverified,
            },
            as_of: r.head.as_of.map(time),
            pending_events: r.pending_events,
            drilldown: DrilldownDto::of(&r.drilldown),
        }
    }
}

impl AlertDto {
    fn of(a: &Alert) -> Self {
        Self {
            kind: a.kind.as_str().to_owned(),
            branch_id: a.branch.as_str().to_owned(),
            since: a.since.map(time),
            detail: a.detail.clone(),
            drilldown: a.drilldown.as_ref().map(DrilldownDto::of),
        }
    }
}

impl ExecutiveOverviewData {
    pub fn of(filter: &ExecutiveFilter, o: &Overview) -> Self {
        Self {
            period: OverviewPeriod {
                from: filter.from.map(|d| d.to_string()),
                to: filter.to.map(|d| d.to_string()),
            },
            kpis: o.kpis.iter().map(MetricDto::of).collect(),
            branches: o.branches.iter().map(BranchRowDto::of).collect(),
            alerts: o.alerts.iter().map(AlertDto::of).collect(),
            definitions: o
                .definitions
                .iter()
                .map(|d| DefinitionDto {
                    key: d.key.to_owned(),
                    th: d.th.to_owned(),
                    en: d.en.to_owned(),
                })
                .collect(),
            buy_reversed_thb: crate::application::executive::metrics::thb(o.buy_reversed_thb),
            sell_reversed_thb: crate::application::executive::metrics::thb(o.sell_reversed_thb),
        }
    }
}
