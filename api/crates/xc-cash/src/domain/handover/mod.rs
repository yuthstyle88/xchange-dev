//! Handover domain model (CA-4 #620; design §4.2).

use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoverStatus {
    Pending,
    Accepted,
    Rejected,
}

impl HandoverStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "ACCEPTED" => Self::Accepted,
            "REJECTED" => Self::Rejected,
            _ => Self::Pending,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Accepted => "ACCEPTED",
            Self::Rejected => "REJECTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub id: String,
    pub branch_id: BranchId,
    pub from_shift_id: String,
    pub to_shift_id: Option<String>,
    pub count_id: String,
    pub receiver_id: String,
    pub status: HandoverStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandoverError {
    #[error("การส่งมอบนี้ไม่อยู่ในสถานะรอรับแล้ว")]
    NotPending,
    #[error("การส่งมอบต้องรับรองผ่านการส่งมอบเท่านั้น")]
    OnlyThroughHandover,
}
