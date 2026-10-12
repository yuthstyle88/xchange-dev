//! `CouponService::issue` (design §6, D-9): server-generated codes, shown once; only `code_hash` is stored.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::coupon::benefit_error::BenefitError;
use crate::domain::coupon::code_hasher::CouponCodeHasher;
use crate::domain::coupon::coupon::{Coupon, CouponStatus};

/// A coupon just issued, with its code (the only time the code is seen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedCoupon {
    pub coupon: Coupon,
    pub code: String,
}

pub struct CouponService {
    pool: PgPool,
    hasher: CouponCodeHasher,
}

impl CouponService {
    pub fn new(pool: PgPool, hasher: CouponCodeHasher) -> Self {
        Self { pool, hasher }
    }

    /// Issues `count` `ISSUED` coupons of `rule_id`, personal when `customer_id` is set; `issued_by` is a
    /// `users.id`.
    pub async fn issue(
        &self,
        rule_id: Uuid,
        count: u32,
        customer_id: Option<&str>,
        expires_at: DateTime<Utc>,
        issued_by: Uuid,
    ) -> Result<Vec<IssuedCoupon>, BenefitError> {
        let (org_id,): (Uuid,) = sqlx::query_as(
            "SELECT cv.organization_id \
             FROM benefit_rules br \
             JOIN campaign_versions cv ON br.campaign_version_id = cv.id \
             WHERE br.id = $1",
        )
        .bind(rule_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(BenefitError::NotFound)?;

        let mut tx = self.pool.begin().await?;
        let mut results = Vec::with_capacity(count as usize);

        for _ in 0..count {
            let coupon_id = Uuid::new_v4();
            let code = format!(
                "CPN-{}",
                Uuid::new_v4().simple().to_string().to_ascii_uppercase()
            );
            let code_hash = self.hasher.hash(&code);

            sqlx::query(
                "INSERT INTO coupons ( \
                     id, organization_id, rule_id, code_hash, customer_id, \
                     expires_at, status, issued_by, issued_at \
                 ) VALUES ($1, $2, $3, $4, $5, $6, 'ISSUED', $7, now())",
            )
            .bind(coupon_id)
            .bind(org_id)
            .bind(rule_id)
            .bind(&code_hash[..])
            .bind(customer_id)
            .bind(expires_at)
            .bind(issued_by)
            .execute(&mut *tx)
            .await?;

            results.push(IssuedCoupon {
                coupon: Coupon {
                    id: coupon_id,
                    organization_id: org_id,
                    rule_id,
                    customer_id: customer_id.map(str::to_owned),
                    expires_at,
                    status: CouponStatus::Issued,
                },
                code,
            });
        }

        tx.commit().await?;
        Ok(results)
    }
}
