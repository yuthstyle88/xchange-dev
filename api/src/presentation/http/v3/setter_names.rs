//! F4/F6 (#6038, design `docs/design/blueprint/user-set-settings.md`): one owner of "user id -> name".

use std::collections::HashMap;

use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};

pub(crate) struct SetterNames;

impl SetterNames {
    pub(crate) async fn of_in(
        tx: &mut Transaction<'_, Postgres>,
        ids: &[String],
    ) -> AppResult<HashMap<String, String>> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id::text, NULLIF(btrim(name), '') FROM users WHERE id::text = ANY($1)",
        )
        .bind(ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .filter_map(|(id, name)| Some((id, name?)))
            .collect())
    }
}
