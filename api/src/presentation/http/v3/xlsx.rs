//! C5 (#1116-style RBAC set, `docs/design/blueprint/user-management-rbac.md` §3.6, §7): the xlsx form of a paged
//! view's export (`?purpose=export&format=xlsx`, through `ExportGate`). One sheet, named after the view; the header
//! is the rows' serde field names in order; strings are always string cells (never a formula), numbers are numbers,
//! booleans are booleans, `null` is an empty cell, an object or an array is its JSON text.

use rust_xlsxwriter::Workbook;
use serde_json::Value;

/// The MIME type of an xlsx answer.
pub const XLSX_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// More rows than this ⇒ `413 exportTooLarge`, nothing written and no audit row.
pub const MAX_EXPORT_ROWS: usize = 50_000;

/// Why a workbook could not be written.
#[derive(Debug)]
pub struct XlsxError(pub String);

impl std::fmt::Display for XlsxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "xlsx: {}", self.0)
    }
}

impl std::error::Error for XlsxError {}

/// One serialised row with its keys in serialisation order. A `serde_json::Value` object cannot carry that order
/// (this crate has no `preserve_order`), and the header must be the row's serde field names in order.
pub type OrderedRow = Vec<(String, Value)>;

struct OrderedMap(OrderedRow);

impl<'de> serde::Deserialize<'de> for OrderedMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;

        impl<'de> serde::de::Visitor<'de> for MapVisitor {
            type Value = OrderedRow;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    out.push(entry);
                }
                Ok(out)
            }
        }

        deserializer.deserialize_map(MapVisitor).map(OrderedMap)
    }
}

/// `row` serialised and read back with its top-level keys in order. Not an object ⇒ `XlsxError`.
pub fn ordered_row<T: serde::Serialize>(row: &T) -> Result<OrderedRow, XlsxError> {
    let json = serde_json::to_string(row).map_err(|e| XlsxError(e.to_string()))?;
    serde_json::from_str::<OrderedMap>(&json)
        .map(|m| m.0)
        .map_err(|e| XlsxError(e.to_string()))
}

/// One workbook with one sheet named `sheet`, from `rows`. The header is the first row's keys in order.
pub fn workbook(sheet: &str, rows: &[OrderedRow]) -> Result<Vec<u8>, XlsxError> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name(sheet).map_err(|e| XlsxError(e.to_string()))?;

    if let Some(first) = rows.first() {
        for (c_idx, (key, _)) in first.iter().enumerate() {
            let col = u16::try_from(c_idx).map_err(|e| XlsxError(e.to_string()))?;
            ws.write_string(0, col, key)
                .map_err(|e| XlsxError(e.to_string()))?;
        }
        for (r_idx, row) in rows.iter().enumerate() {
            let row_num = u32::try_from(r_idx + 1).map_err(|e| XlsxError(e.to_string()))?;
            for (c_idx, (_, val)) in row.iter().enumerate() {
                let col = u16::try_from(c_idx).map_err(|e| XlsxError(e.to_string()))?;
                match val {
                    Value::Null => {}
                    Value::Bool(b) => {
                        ws.write_boolean(row_num, col, *b)
                            .map_err(|e| XlsxError(e.to_string()))?;
                    }
                    Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            ws.write_number(row_num, col, f)
                                .map_err(|e| XlsxError(e.to_string()))?;
                        } else {
                            ws.write_string(row_num, col, n.to_string())
                                .map_err(|e| XlsxError(e.to_string()))?;
                        }
                    }
                    Value::String(s) => {
                        ws.write_string(row_num, col, s)
                            .map_err(|e| XlsxError(e.to_string()))?;
                    }
                    Value::Array(_) | Value::Object(_) => {
                        let text =
                            serde_json::to_string(val).map_err(|e| XlsxError(e.to_string()))?;
                        ws.write_string(row_num, col, &text)
                            .map_err(|e| XlsxError(e.to_string()))?;
                    }
                }
            }
        }
    }

    wb.save_to_buffer().map_err(|e| XlsxError(e.to_string()))
}
