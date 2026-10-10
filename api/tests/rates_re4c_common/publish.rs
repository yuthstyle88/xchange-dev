// Ticket #757 (RE-4c) — publishing a rate set through the rate engine, and the newest `rates`
// rows of a branch as its next publication. Written before the implementation by the ticket
// author (claude/e5ab6c). Do not edit, delete or weaken.
//
// Not a module: `include!`d by `save_common` (as its inline `rates_re4c_common`) and by
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
        .submit(&mut tx, &b, &owner, &draft.id, &re4c_unique_key())
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
