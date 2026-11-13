//! `PgDisplaySettingsStore`: `rate_display_settings` (pricing-tiers.md §15.2, §15.8).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a4c's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::display_settings::{
    DisplayFrom, DisplaySettings, DisplaySettingsStore,
};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgDisplaySettingsStore;

async fn fetch_row(
    conn: &mut PgConnection,
    branch: &str,
) -> Result<Option<DisplaySettings>, RateEngineError> {
    let row = sqlx::query(
        "SELECT tiers, featured::text[] AS featured, highlight, all_market \
         FROM rate_display_settings \
         WHERE branch_id = $1",
    )
    .bind(branch)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| RateEngineError::Database(e.to_string()))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let tiers_raw: Vec<i16> = row.get("tiers");
    let tiers = tiers_raw
        .into_iter()
        .map(|t| {
            u8::try_from(t).map_err(|_| {
                RateEngineError::Internal(format!(
                    "rate_display_settings({branch}) invalid tier {t}"
                ))
            })
        })
        .collect::<Result<Vec<u8>, _>>()?;

    let featured: Vec<String> = row.get("featured");

    Ok(Some(DisplaySettings {
        tiers,
        featured,
        highlight: row.get("highlight"),
        all_market: row.get("all_market"),
    }))
}

impl DisplaySettingsStore for PgDisplaySettingsStore {
    fn effective_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, (DisplaySettings, DisplayFrom)> {
        Box::pin(async move {
            if let Some(settings) = fetch_row(conn, branch).await? {
                return Ok((settings, DisplayFrom::Branch));
            }
            if branch != "MAIN"
                && let Some(settings) = fetch_row(conn, "MAIN").await?
            {
                return Ok((settings, DisplayFrom::Main));
            }
            Ok((DisplaySettings::not_set(), DisplayFrom::Default))
        })
    }

    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        settings: &'a DisplaySettings,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let tiers: Vec<i16> = settings.tiers.iter().map(|&t| i16::from(t)).collect();
            sqlx::query(
                "INSERT INTO rate_display_settings \
                 (branch_id, tiers, featured, highlight, all_market, updated_by, updated_at) \
                 VALUES ($1, $2, $3::char(3)[], $4, $5, $6, $7) \
                 ON CONFLICT (branch_id) DO UPDATE \
                 SET tiers = EXCLUDED.tiers, \
                     featured = EXCLUDED.featured, \
                     highlight = EXCLUDED.highlight, \
                     all_market = EXCLUDED.all_market, \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(branch)
            .bind(tiers)
            .bind(&settings.featured)
            .bind(settings.highlight)
            .bind(settings.all_market)
            .bind(by)
            .bind(at)
            .execute(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn clear_in<'a>(&'a self, conn: &'a mut PgConnection, branch: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("DELETE FROM rate_display_settings WHERE branch_id = $1")
                .bind(branch)
                .execute(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>> {
        Box::pin(async move {
            let row = sqlx::query("SELECT MAX(updated_at) AS newest FROM rate_display_settings")
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(row.get("newest"))
        })
    }
}
