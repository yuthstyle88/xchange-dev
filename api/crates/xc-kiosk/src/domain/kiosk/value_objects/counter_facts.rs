use std::fmt;

use xc_kernel::money::{MinorUnit, Rate, UnitSize};

/// The identifier of a published rate line (`published_rate_lines.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineId(pub u128);

impl LineId {
    pub fn parse_uuid(s: &str) -> Option<Self> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return None;
        }
        u128::from_str_radix(&hex, 16).ok().map(LineId)
    }
}

impl fmt::Display for LineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (self.0 >> 96) as u32,
            ((self.0 >> 80) & 0xffff) as u16,
            ((self.0 >> 64) & 0xffff) as u16,
            ((self.0 >> 48) & 0xffff) as u16,
            (self.0 & 0xffff_ffff_ffff) as u64,
        )
    }
}

/// One published line as the counter holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterLine {
    pub line_id: LineId,
    pub rate: Rate,
    pub unit_size: UnitSize,
    pub minor_unit: MinorUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterFacts {
    pub open: bool,
    pub line: Option<CounterLine>,
}
