use std::fmt;

use xc_kernel::money::{MinorUnit, Rate, UnitSize};

/// The identifier of a published rate line (`published_rate_lines.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineId(pub u128);

impl LineId {
    pub fn parse_uuid(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 36 || b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        let mut val = 0u128;
        for (i, &byte) in b.iter().enumerate() {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                continue;
            }
            let digit = match byte {
                b'0'..=b'9' => (byte - b'0') as u128,
                b'a'..=b'f' => (byte - b'a' + 10) as u128,
                b'A'..=b'F' => (byte - b'A' + 10) as u128,
                _ => return None,
            };
            val = (val << 4) | digit;
        }
        Some(LineId(val))
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
