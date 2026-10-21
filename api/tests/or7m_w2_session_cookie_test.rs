//! W2-T2 (#4015, parent #1027): pins the four `cookie()` rules (`api/src/presentation/extractors/actor.rs:25`)
//! on `__Host-xc_session` through `POST /api/auth/session`.

mod auth_http_common;
mod common;

use auth_http_common::{Call, Desk, send};
use serde_json::{Value, json};

async fn seeded_user_id(desk: &Desk, name: &str) -> String {
    sqlx::query_scalar("SELECT id::text FROM users WHERE name = $1")
        .bind(name)
        .fetch_one(&desk.pool)
        .await
        .expect("seeded user id")
}

fn session_call(cookie_line: String) -> Call {
    let mut call = Call::post("/api/auth/session", json!({}));
    call.cookie = Some(cookie_line);
    call
}

#[actix_web::test]
async fn row_0_baseline_session_cookie_resolves_the_user() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let token = desk.staff_session("somchai", "TELLER", "MAIN").await;
    let user_id = seeded_user_id(&desk, "somchai").await;

    let answer = send(&app, session_call(format!("__Host-xc_session={token}"))).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["user"]["id"],
        user_id.as_str(),
        "baseline session cookie resolves the seeded user: {}",
        answer.text
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn row_1_spaces_around_the_cookie_pair_are_trimmed() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let token = desk.staff_session("somchai", "TELLER", "MAIN").await;
    let user_id = seeded_user_id(&desk, "somchai").await;

    let answer = send(
        &app,
        session_call(format!("theme=dark;  __Host-xc_session={token} ;lang=th")),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["user"]["id"],
        user_id.as_str(),
        "spaces around the pair are trimmed: {}",
        answer.text
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn row_2_session_cookie_is_found_among_other_cookies() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let token = desk.staff_session("somchai", "TELLER", "MAIN").await;
    let user_id = seeded_user_id(&desk, "somchai").await;

    let answer = send(
        &app,
        session_call(format!("a=b; __Host-xc_session={token}; c=d")),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["user"]["id"],
        user_id.as_str(),
        "found among other cookies: {}",
        answer.text
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn row_3_first_cookie_of_that_name_counts() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let token = desk.staff_session("somchai", "TELLER", "MAIN").await;

    let answer = send(
        &app,
        session_call(format!("__Host-xc_session=junk; __Host-xc_session={token}")),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["user"],
        Value::Null,
        "the first cookie of that name counts: {}",
        answer.text
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn row_4_name_that_merely_ends_like_ours_is_not_ours() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let token = desk.staff_session("somchai", "TELLER", "MAIN").await;

    let answer = send(&app, session_call(format!("x__Host-xc_session={token}"))).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["user"],
        Value::Null,
        "a name that merely ends like ours is not ours: {}",
        answer.text
    );

    drop(app);
    desk.drop_schema().await;
}
