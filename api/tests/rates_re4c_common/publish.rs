// Ticket #757 (RE-4c) — publishing a rate set through the rate engine, and the newest `rates`
// rows of a branch as its next publication. Written before the implementation by the ticket
// author (claude/e5ab6c). Do not edit, delete or weaken.
//
// Re-expected for RE-7 #604 by the ticket author: migration 6 drops `rates`, so a harness can no
// longer append a row and publish "the newest rows". `set_counter_rate` (per 1, as the retired
// direct save took it) and `publish_over_current` (per `unit_size`) state the rates they publish
// and carry the rest of the branch's current publication over, which is the rate set the newest
// rows made when every rate was set this way. `publish_newest_rates` is kept only for its callers
// outside this helper's files (`snapshot_common`, `rpc_common`, `snapshot_test`,
// `v3_desk_config_view_test`); it reads `rates`, so it fails once migration 6 has run.
//
// Not a module: `include!`d by `save_common` (as its inline `rates_re4c_common`), by
// `rates_re4e_publish` (as its inline `engine`, RE-7 #604) and by
// `rates_re4c_common/mod.rs` (loaded by `snapshot_common`), so that no test binary loads one
// file as a module twice (clippy::duplicate_mod) — not this one, and not `rates_re4_common`,
// which `kiosk_http_common` and several test files load themselves. The rate engine is driven as
// `rates_re4_common::publish` drives it (RE-4a, merged); every path is written in full so the
// including module's own imports neither clash nor are needed.

pub fn d(text: &str) -> ::rust_decimal::Decimal {
    <::rust_decimal::Decimal as ::std::str::FromStr>::from_str(text)
        .unwrap_or_else(|e| panic!("{text} is a decimal: {e}"))
}

/// The publication a branch points at after a publish: `(id, version)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub id: String,
    pub version: i32,
}

/// One line of a branch's current publication, as stored (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub buy: Option<::rust_decimal::Decimal>,
    pub sell: Option<::rust_decimal::Decimal>,
    pub unit_size: ::rust_decimal::Decimal,
}

/// Publish `lines` (`(currency, buy, sell)`, each rate per the currency's `unit_size`) as the
/// next publication of `branch`, as the owner. Answers the new publication.
pub async fn publish(
    pool: &::sqlx::PgPool,
    branch: &str,
    lines: &[(&str, &str, &str)],
) -> Published {
    let owner = ::xc_auth::domain::policy::value_objects::actor::Actor::owner("owner");
    // OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `submit` takes the
    // owner's B1-mapped user, EXECUTIVE + ADMIN organisation-wide.
    let owner_user = ::xc_auth::domain::access::Actor::new(
        "owner",
        "00000000-0000-0000-0000-000000000001",
        vec![
            ::xc_auth::domain::access::ScopedRole::new(
                ::xc_auth::domain::access::Role::Executive,
                None,
                ::chrono::DateTime::<::chrono::Utc>::UNIX_EPOCH,
                None,
            ),
            ::xc_auth::domain::access::ScopedRole::new(
                ::xc_auth::domain::access::Role::Admin,
                None,
                ::chrono::DateTime::<::chrono::Utc>::UNIX_EPOCH,
                None,
            ),
        ],
        None,
    );
    let b = ::xc_kernel::ids::BranchId::new(branch);
    let drafts = ::xc_rate::application::draft::rate_draft_service::RateDraftService::new(
        pool.clone(),
        ::xchange::presentation::http::v3::commands::make_approval_service(pool.clone()),
    );
    let publisher =
        ::xc_rate::application::publication::publish_service::PublishService::new(pool.clone());
    let draft_lines: Vec<::xc_rate::domain::draft::rate_draft::DraftLine> = lines
        .iter()
        .map(|(code, buy, sell)| {
            ::xc_rate::domain::draft::rate_draft::DraftLine::new(
                ::xc_kernel::money::CurrencyCode::parse(code)
                    .unwrap_or_else(|_| panic!("{code} is a currency")),
                ::xc_rate::domain::draft::rate_draft::LineSide::Manual(
                    ::xc_kernel::money::Rate::parse(buy).unwrap_or_else(|e| panic!("{buy}: {e:?}")),
                ),
                ::xc_rate::domain::draft::rate_draft::LineSide::Manual(
                    ::xc_kernel::money::Rate::parse(sell)
                        .unwrap_or_else(|e| panic!("{sell}: {e:?}")),
                ),
                Some("published by the RE-4c test".to_owned()),
            )
        })
        .collect();

    let mut tx = pool.begin().await.expect("begin");
    let draft = drafts
        .create(&mut tx, &b, &owner, draft_lines)
        .await
        .unwrap_or_else(|e| panic!("create the draft: {}", e.code()));
    tx.commit().await.expect("commit the draft");

    let mut tx = pool.begin().await.expect("begin");
    drafts
        .submit(&mut tx, &b, &owner_user, &draft.id, &re4c_unique_key())
        .await
        .unwrap_or_else(|e| panic!("submit the draft: {}", e.code()));
    tx.commit().await.expect("commit the submit");

    let expected: Option<i32> = sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the current version");

    let mut tx = pool.begin().await.expect("begin");
    let published = publisher
        .publish(&mut tx, &b, &owner, &draft.id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
    tx.commit().await.expect("commit the publish");

    let now = current(pool, branch)
        .await
        .expect("the branch points at it");
    assert_eq!(
        now.version, published.version,
        "the branch points at the publication just made"
    );
    now
}

/// The branch's current publication, or `None` before its first.
pub async fn current(pool: &::sqlx::PgPool, branch: &str) -> Option<Published> {
    sqlx::query_as::<_, (String, i32)>(
        "SELECT p.id::text, p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the current publication")
    .map(|(id, version)| Published { id, version })
}

/// The `ALL` line of `code` in `branch`'s current publication.
pub async fn line(pool: &::sqlx::PgPool, branch: &str, code: &str) -> Line {
    let (id, buy, sell, unit_size): (
        String,
        Option<::rust_decimal::Decimal>,
        Option<::rust_decimal::Decimal>,
        ::rust_decimal::Decimal,
    ) = sqlx::query_as(
        "SELECT l.id::text, l.buy_rate, l.sell_rate, l.unit_size \
             FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL'",
    )
    .bind(branch)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the current {code} line of {branch}: {e}"));
    Line {
        id,
        buy,
        sell,
        unit_size,
    }
}

fn re4c_unique_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    format!(
        "re4c-key-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Publish `branch`'s newest `rates` row of every currency (per 1, as the direct save appends
/// it), × `currencies.unit_size` as migration 4 converts them, as the branch's next
/// publication: the rate set the counter quoted before RE-4. For the harnesses that set a
/// counter rate by appending to `rates` (D-3 makes the publication the counter rate).
pub async fn publish_newest_rates(pool: &::sqlx::PgPool, branch: &str) -> Published {
    let newest: Vec<(String, ::rust_decimal::Decimal, ::rust_decimal::Decimal)> = ::sqlx::query_as(
        "SELECT DISTINCT ON (r.code) trim(r.code), r.buy * c.unit_size, r.sell * c.unit_size \
             FROM rates r JOIN currencies c ON c.code = r.code \
             WHERE r.branch_id = $1 ORDER BY r.code, r.id DESC",
    )
    .bind(branch)
    .fetch_all(pool)
    .await
    .expect("read the branch's newest rates");
    let texts: Vec<(String, String, String)> = newest
        .into_iter()
        .map(|(code, buy, sell)| {
            (
                code,
                buy.normalize().to_string(),
                sell.normalize().to_string(),
            )
        })
        .collect();
    let lines: Vec<(&str, &str, &str)> = texts
        .iter()
        .map(|(c, b, s)| (c.as_str(), b.as_str(), s.as_str()))
        .collect();
    publish(pool, branch, &lines).await
}

/// RE-7 #604: publish `lines` (`(currency, buy, sell)`, per the currency's `unit_size`) as the
/// next publication of `branch`, with every other `ALL` line of its current publication carried
/// over unchanged. A line with an empty side is not carried (the draft takes a rate per side).
pub async fn publish_over_current(
    pool: &::sqlx::PgPool,
    branch: &str,
    lines: &[(&str, &str, &str)],
) -> Published {
    let kept: Vec<(String, ::rust_decimal::Decimal, ::rust_decimal::Decimal)> = ::sqlx::query_as(
        "SELECT trim(l.currency_code), l.buy_rate, l.sell_rate \
             FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1 AND l.denomination_group = 'ALL' \
               AND l.buy_rate IS NOT NULL AND l.sell_rate IS NOT NULL \
             ORDER BY 1",
    )
    .bind(branch)
    .fetch_all(pool)
    .await
    .expect("read the branch's current lines");
    let mut texts: Vec<(String, String, String)> = kept
        .into_iter()
        .filter(|(code, _, _)| lines.iter().all(|(c, _, _)| c != code))
        .map(|(code, buy, sell)| {
            (
                code,
                buy.normalize().to_string(),
                sell.normalize().to_string(),
            )
        })
        .collect();
    texts.extend(
        lines
            .iter()
            .map(|(c, b, s)| ((*c).to_owned(), (*b).to_owned(), (*s).to_owned())),
    );
    // in code order, as the newest `rates` rows were published (`DISTINCT ON (r.code)`)
    texts.sort();
    let all: Vec<(&str, &str, &str)> = texts
        .iter()
        .map(|(c, b, s)| (c.as_str(), b.as_str(), s.as_str()))
        .collect();
    publish(pool, branch, &all).await
}

/// RE-7 #604: the counter rate of `code` in `branch` from now on, given per 1 as the retired
/// direct save took it: published × `currencies.unit_size` (JPY 0.2 per 1 is 20 per 100), as
/// migration 4 converted a `rates` row, over the branch's current publication.
pub async fn set_counter_rate(
    pool: &::sqlx::PgPool,
    branch: &str,
    code: &str,
    buy_per_one: &str,
    sell_per_one: &str,
) -> Published {
    let unit: ::rust_decimal::Decimal =
        ::sqlx::query_scalar("SELECT unit_size FROM currencies WHERE code = $1")
            .bind(code)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|e| panic!("the unit size of {code}: {e}"));
    let buy = (d(buy_per_one) * unit).normalize().to_string();
    let sell = (d(sell_per_one) * unit).normalize().to_string();
    publish_over_current(pool, branch, &[(code, buy.as_str(), sell.as_str())]).await
}
