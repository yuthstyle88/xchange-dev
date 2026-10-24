//! Denomination lookups for cash flows.

use std::collections::HashMap;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};

/// Look up face values and currency codes for denomination IDs.
///
/// An ID whose `face_value` is NULL is refused with `AppError::NotFound { entity: "denomination", id }`
/// naming the ID.
pub(crate) async fn face_values_in(
    tx: &mut Transaction<'_, Postgres>,
    ids: &[i64],
) -> AppResult<HashMap<i64, (Decimal, String)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<(i64, Option<Decimal>, String)> = sqlx::query_as(
        "SELECT id, face_value, currency_code FROM denominations WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;

    let mut map: HashMap<i64, (Option<Decimal>, String)> = HashMap::with_capacity(rows.len());
    for (id, face_value, currency_code) in rows {
        map.insert(id, (face_value, currency_code));
    }

    for &id in ids {
        if let Some((face_value, _)) = map.get(&id) {
            if face_value.is_none() {
                return Err(AppError::NotFound {
                    entity: "denomination",
                    id: id.to_string(),
                });
            }
        }
    }

    let result = map
        .into_iter()
        .filter_map(|(id, (fv, cur))| fv.map(|v| (id, (v, cur))))
        .collect();

    Ok(result)
}
