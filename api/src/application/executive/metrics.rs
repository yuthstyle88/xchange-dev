//! A01's figures (design `executive-finance.md` §3, D-2): every figure is a [`Metric`] with a definition key, and
//! either a value or the reason it is not available yet. No figure is a stand-in number.

use rust_decimal::Decimal;

use super::drilldown::Drilldown;

/// Why a figure has no value yet (design §3). On the wire camelCase: `awaitsLedger`, ….
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricUnavailable {
    /// Realized margin needs the ledger's cost allocation (LG-4).
    AwaitsLedger,
    /// Booking conversion needs the booking design's appointment day (#555).
    AwaitsBookingDesign,
    /// Commission comes from Broker-Platform (§8.3).
    AwaitsBroker,
    /// Promotion and reward expense needs campaigns (§05).
    AwaitsCampaigns,
}

impl MetricUnavailable {
    pub fn as_str(self) -> &'static str {
        match self {
            MetricUnavailable::AwaitsLedger => "awaitsLedger",
            MetricUnavailable::AwaitsBookingDesign => "awaitsBookingDesign",
            MetricUnavailable::AwaitsBroker => "awaitsBroker",
            MetricUnavailable::AwaitsCampaigns => "awaitsCampaigns",
        }
    }
}

/// One figure: a value, or the reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    pub key: &'static str,
    pub value: Option<String>,
    pub unavailable: Option<MetricUnavailable>,
    pub drilldown: Option<Drilldown>,
}

impl Metric {
    /// A figure with its value.
    pub fn value(key: &'static str, value: String, drilldown: Option<Drilldown>) -> Metric {
        Metric {
            key,
            value: Some(value),
            unavailable: None,
            drilldown,
        }
    }

    /// A figure whose source does not exist yet.
    pub fn unavailable(key: &'static str, reason: MetricUnavailable) -> Metric {
        Metric {
            key,
            value: None,
            unavailable: Some(reason),
            drilldown: None,
        }
    }
}

pub const BUY_VOLUME_THB: &str = "buy_volume_thb";
pub const SELL_VOLUME_THB: &str = "sell_volume_thb";
pub const REALIZED_MARGIN_THB: &str = "realized_margin_thb";
pub const REPEAT_CUSTOMERS: &str = "repeat_customers";
pub const BOOKING_CONVERSION: &str = "booking_conversion";
pub const COMMISSION_THB: &str = "commission_thb";
pub const AVAILABLE_CASH: &str = "available_cash";
pub const IN_TRANSIT: &str = "in_transit";
pub const DATA_FRESHNESS: &str = "data_freshness";

/// The six KPIs of E-1, in the §04 wireframe's order.
pub const KPI_KEYS: [&str; 6] = [
    BUY_VOLUME_THB,
    SELL_VOLUME_THB,
    REALIZED_MARGIN_THB,
    REPEAT_CUSTOMERS,
    BOOKING_CONVERSION,
    COMMISSION_THB,
];

/// The §11 text of a figure key (§10 "metric definition").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Definition {
    pub key: &'static str,
    pub th: &'static str,
    pub en: &'static str,
}

/// The definition of every key A01 answers (design §3).
pub fn definitions() -> &'static [Definition] {
    &[
        Definition {
            key: BUY_VOLUME_THB,
            th: "ยอดซื้อ: มูลค่าบาทของรายการที่ร้านซื้อ (BUY) ที่ทำรายการสำเร็จ ยอดรวมก่อนหักค่าธรรมเนียม ไม่ใช่รายได้",
            en: "Buy volume: THB equivalent of completed lines with shop side BUY, gross; volume is not revenue",
        },
        Definition {
            key: SELL_VOLUME_THB,
            th: "ยอดขาย: มูลค่าบาทของรายการที่ร้านขาย (SELL) ที่ทำรายการสำเร็จ ยอดรวมก่อนหักค่าธรรมเนียม ไม่ใช่รายได้",
            en: "Sell volume: THB equivalent of completed lines with shop side SELL, gross; volume is not revenue",
        },
        Definition {
            key: REALIZED_MARGIN_THB,
            th: "ส่วนต่างรับรู้: เงินบาทที่ได้จากการขาย หักต้นทุนเงินตราที่จัดสรร การซื้อไม่รับรู้ส่วนต่าง",
            en: "Realized margin: THB received on SELL minus the allocated FX cost; BUY recognises no margin",
        },
        Definition {
            key: REPEAT_CUSTOMERS,
            th: "ลูกค้าซ้ำ: ลูกค้าที่ทำรายการสำเร็จในช่วงเวลานี้ และเคยทำรายการสำเร็จมาก่อนที่สาขาใดก็ได้",
            en: "Repeat customers: customers with a completed transaction in the period and an earlier completed one at any branch",
        },
        Definition {
            key: BOOKING_CONVERSION,
            th: "จอง → สำเร็จ: การจองที่สำเร็จ ÷ การจองที่ยืนยัน ของวันนัดเดียวกัน",
            en: "Booking conversion: completed bookings divided by confirmed bookings of the same appointment-day cohort",
        },
        Definition {
            key: COMMISSION_THB,
            th: "ค่าคอม: ค่าคอมมิชชันของช่วงเวลา แยกตามสถานะของคอมมิชชัน",
            en: "Commission: commission expense of the period, split by commission state",
        },
        Definition {
            key: AVAILABLE_CASH,
            th: "เงินพร้อมใช้งาน (บาท): เงินในลิ้นชักของกะที่เปิดและในตู้เซฟ หักยอดที่จองไว้ ไม่รวมเงินระหว่างทาง",
            en: "Available cash (THB): baht in open shifts' drawers and the safe, less reservations; in transit apart",
        },
        Definition {
            key: IN_TRANSIT,
            th: "เงินระหว่างทาง (บาท): เงินบาทในการโอนที่ส่งออกจากสาขาแล้วแต่ยังไม่ได้รับ",
            en: "In transit (THB): baht in transfers dispatched from the branch and not yet received",
        },
        Definition {
            key: DATA_FRESHNESS,
            th: "เวลา sync: เวลาที่ข้อมูลของสาขาเปลี่ยนล่าสุด และจำนวนรายการที่ยังไม่เสร็จ",
            en: "Data freshness: when the branch's data last changed, and its unfinished transactions",
        },
    ]
}

/// A THB amount as the wire writes it: a decimal string with two places.
pub fn thb(amount: Decimal) -> String {
    let mut a = amount.round_dp(2);
    a.rescale(2);
    a.to_string()
}
