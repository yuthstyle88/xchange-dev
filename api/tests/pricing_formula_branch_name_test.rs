//! OL-8 row 54: the `pricing_formula` view names each branch (`branches.name`), so the
//! "สาขาใช้สูตรไหน" table can print a name instead of the id.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, owner, view};
use pricing_formula_api::*;
use serde_json::{Value, json};

async fn rename(room: &Room, branch: &str, name: &str) {
    sqlx::query("UPDATE branches SET name = $2 WHERE id = $1")
        .bind(branch)
        .bind(name)
        .execute(&room.pool)
        .await
        .expect("rename branch");
}

#[actix_web::test]
async fn each_branch_row_carries_the_branch_name_beside_its_id() {
    let room = Room::start().await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    rename(&room, "MAIN", "สำนักงานใหญ่").await;
    rename(&room, PFB, "สาขาสยาม ชั้น 2").await;
    let data = view(&room, &owner(), "pricing_formula", "MAIN")
        .await
        .ok("view")["data"]
        .clone();
    let rows: Vec<(Value, Value)> = data["branches"]
        .as_array()
        .expect("branches array")
        .iter()
        .map(|b| (b["branch"].clone(), b["name"].clone()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (json!("MAIN"), json!("สำนักงานใหญ่")),
            (json!(PFB), json!("สาขาสยาม ชั้น 2")),
        ]
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_row_has_exactly_branch_name_source_and_own() {
    let room = Room::start().await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    rename(&room, "MAIN", "สำนักงานใหญ่").await;
    let data = view(&room, &owner(), "pricing_formula", "MAIN")
        .await
        .ok("view")["data"]
        .clone();
    let mut keys: Vec<String> = data["branches"][0]
        .as_object()
        .expect("branch row object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["branch", "name", "own", "source"]);
    room.finish().await;
}
