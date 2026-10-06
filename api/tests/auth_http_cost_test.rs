//! Ticket T4c — the account service the server builds hashes with the cost for real accounts.
//!
//! `PasswordHasher::with_cost` is public (the tests of T4b use it to stay fast); the service
//! `auth::configure` builds must use `PasswordHasher::new()`. This holds an employee created
//! through `/api/employees` to the same check `create_owner_test` makes of the owner.
//! Needs `TEST_DATABASE_URL` like `auth_http_test`; it fails without it, it does not skip.

mod auth_http_common;
mod common;

use auth_http_common::*;
use serde_json::json;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;

const FIRST_PASSWORD: &str = "first-password-1";

#[actix_web::test]
async fn an_employee_created_through_the_route_is_hashed_with_the_production_cost() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;

    let made = send(
        &app,
        Call::post(
            "/api/employees",
            json!({ "action": "create", "payload": {
                "username": "costly", "name": "Costly", "role": "RATES", "branchId": "MAIN",
                "password": FIRST_PASSWORD } }),
        )
        .owner(&owner),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.text);

    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM employees WHERE username = 'costly'")
            .fetch_one(&desk.pool)
            .await
            .expect("the employee row");
    assert!(hash.starts_with("$argon2id$v=19$"), "stored: {hash}");
    let memory: u32 = hash
        .split('$')
        .nth(3)
        .and_then(|params| params.split(',').find_map(|p| p.strip_prefix("m=")))
        .and_then(|m| m.parse().ok())
        .unwrap_or_else(|| panic!("the memory cost in {hash}"));
    assert!(
        memory >= 19_456,
        "the route hashes with the cost for real accounts: {hash}"
    );
    assert!(
        PasswordHasher::new().verify(FIRST_PASSWORD, &hash),
        "the stored hash is of the password that was sent"
    );

    drop(app);
    desk.drop_schema().await;
}
