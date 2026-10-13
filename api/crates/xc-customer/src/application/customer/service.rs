//! Customer service.

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use sqlx::Transaction;
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Target};
use xc_auth::domain::access::permission::Permission;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, Sealed};

use crate::domain::customer::commands::reserve_customer::ReserveCustomer;
use crate::domain::customer::entities::customer::Customer;
use crate::domain::customer::entities::customer_identity::CustomerIdentity;
use crate::domain::customer::entities::reservation::Reservation;
use crate::domain::customer::repositories::customer_repository::CustomerRepository;
use crate::domain::customer::value_objects::customer_details::CustomerDetails;
use crate::infrastructure::customer::customer_repository::PgCustomerRepository;
use crate::pii_keys::get_pii_keys;

pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const COUNTER_EXHAUSTED: &str = "ตัวนับรหัสไม่ถูกต้อง";
pub const INVALID_NATIONALITY: &str = "สัญชาติไม่ถูกต้อง";
pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";
pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
pub const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReservedCustomerNumber {
    Existing(String),
    New {
        customer_id: String,
        created_at: DateTime<Utc>,
    },
}

#[derive(Clone)]
pub struct CustomerService {
    branch_tx: BranchTx,
    repo: PgCustomerRepository,
}

impl CustomerService {
    pub fn new(branch_tx: BranchTx) -> Self {
        let repo = PgCustomerRepository::new(branch_tx.pool().clone());
        Self { branch_tx, repo }
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub(crate) async fn reserve_customer_number_in_tx(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        org_id: uuid::Uuid,
        lookup_hash: &[u8],
        mode: Mode,
        now: DateTime<Utc>,
    ) -> AppResult<ReservedCustomerNumber> {
        let lock_key = i64::from_be_bytes(lookup_hash[0..8].try_into().unwrap());
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;

        if let Some(existing_id) = Self::find_existing_in_tx(tx, org_id, lookup_hash).await? {
            return Ok(ReservedCustomerNumber::Existing(existing_id));
        }

        let counter_key = match mode {
            Mode::Live => "customer-LIVE",
            Mode::Demo => "customer-DEMO",
        };
        let next_val = PgCustomerRepository::increment_counter_in_tx(tx, counter_key)
            .await?
            .ok_or_else(|| AppError::Domain(COUNTER_EXHAUSTED.to_owned()))?;

        let customer_id = match mode {
            Mode::Live => format!("CX-{next_val:08}"),
            Mode::Demo => format!("TEST-CX-{next_val:08}"),
        };

        let created_at = Utc
            .timestamp_millis_opt(now.timestamp_millis())
            .single()
            .unwrap_or(now);

        Ok(ReservedCustomerNumber::New {
            customer_id,
            created_at,
        })
    }

    pub async fn reserve(
        &self,
        branch: &BranchId,
        cmd: ReserveCustomer,
        now: DateTime<Utc>,
    ) -> AppResult<Reservation> {
        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let details = CustomerDetails::parse(&cmd)?;
        let org_id = self.repo.organization_of_branch(branch).await?;

        let keys = get_pii_keys()?;
        let normalized = xc_kernel::pii::normalize(
            IdentityType::Passport,
            &details.passport_no,
            Some(&details.nationality),
        )
        .map_err(|e| match e {
            xc_kernel::pii::PiiError::InvalidNationality => {
                AppError::Domain(INVALID_NATIONALITY.to_owned())
            }
            _ => AppError::Domain(BAD_TEXT.to_owned()),
        })?;

        let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Passport, &normalized);

        self.branch_tx
            .run(branch, move |tx| {
                let keys = keys.clone();
                let details = details.clone();
                Box::pin(async move {
                    let reserved = Self::reserve_customer_number_in_tx(
                        tx,
                        org_id,
                        lookup_hash.as_slice(),
                        details.mode,
                        now,
                    )
                    .await?;

                    let (customer_id, created_at) = match reserved {
                        ReservedCustomerNumber::Existing(customer_id) => {
                            return Ok(Reservation {
                                customer_id,
                                existing: true,
                            });
                        }
                        ReservedCustomerNumber::New {
                            customer_id,
                            created_at,
                        } => (customer_id, created_at),
                    };

                    let cipher = PiiCipher::new(keys);
                    let aad = format!("{customer_id}PASSPORT");
                    let payload = serde_json::json!({
                        "number": details.passport_no,
                        "nationality": details.nationality,
                        "birthDate": details.birth_date.map(|d| d.to_string()),
                        "expiry": details.passport_expiry.map(|d| d.to_string()),
                        "nameOnDocument": details.name,
                    });
                    let sealed = cipher.encrypt(aad.as_bytes(), payload.to_string().as_bytes());

                    let new_identity = CustomerIdentity {
                        id: uuid::Uuid::new_v4(),
                        organization_id: org_id,
                        customer_id: customer_id.clone(),
                        r#type: IdentityType::Passport,
                        encrypted_value: sealed.bytes,
                        key_id: sealed.key_id,
                        lookup_hash: lookup_hash.to_vec(),
                        nationality: Some(details.nationality.clone()),
                        verified_at: None,
                        status: "ACTIVE".to_string(),
                        created_at,
                        updated_at: None,
                    };

                    PgCustomerRepository::insert_customer_and_identity_in_tx(
                        tx,
                        org_id,
                        &customer_id,
                        &details.name,
                        details.mode,
                        created_at,
                        &new_identity,
                    )
                    .await?;

                    Ok(Reservation {
                        customer_id,
                        existing: false,
                    })
                })
            })
            .await
    }

    pub async fn find(&self, branch: &BranchId, id: &str) -> AppResult<Option<Customer>> {
        self.repo.find(branch, id).await
    }

    pub async fn list_live(&self, branch: &BranchId) -> AppResult<Vec<Customer>> {
        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }
        self.repo.list_live(branch).await
    }

    pub async fn confirm_in(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
    ) -> AppResult<()> {
        PgCustomerRepository::confirm_customer_in_tx(tx, branch, customer_id, details).await
    }

    /// TX-4 #672 Q1 = B: the exchange's customer, read without decrypting. Only the clear
    /// columns are read (`customers.mode`, `.display_name`, the ACTIVE PASSPORT identity's
    /// `nationality`); a value is shown only through `reveal` (K8).
    /// Refuses a customer that is not the branch's organisation's (or no such customer) as
    /// `outOfScope`, a CLOSED one as `validationFailed`, and one with no ACTIVE PASSPORT identity
    /// with [`NOT_THIS_PASSPORT`].
    pub async fn exchange_customer_in(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
    ) -> AppResult<ExchangeCustomer> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT c.mode, c.display_name, c.status FROM customers c \
             JOIN branches b ON b.organization_id = c.organization_id \
             WHERE b.id = $1 AND c.id = $2",
        )
        .bind(branch.as_str())
        .bind(customer_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        let Some((mode, display_name, status)) = row else {
            return Err(AppError::Domain(format!(
                "outOfScope: customer {customer_id} is not this branch's organisation's"
            )));
        };
        if status == "CLOSED" {
            return Err(AppError::Domain(format!(
                "validationFailed: customer {customer_id} is closed"
            )));
        }
        let mode = match mode.as_str() {
            "LIVE" => Mode::Live,
            "DEMO" => Mode::Demo,
            other => return Err(AppError::Internal(format!("customers.mode {other}"))),
        };
        let nationality: Option<Option<String>> = sqlx::query_scalar(
            "SELECT nationality FROM customer_identities \
             WHERE customer_id = $1 AND type = 'PASSPORT' AND status = 'ACTIVE' LIMIT 1",
        )
        .bind(customer_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        let Some(nationality) = nationality else {
            return Err(AppError::Domain(NOT_THIS_PASSPORT.to_owned()));
        };
        Ok(ExchangeCustomer {
            id: customer_id.to_owned(),
            mode,
            display_name,
            nationality: nationality.unwrap_or_default(),
        })
    }

    pub async fn record_exchange_in(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
        document_ref: &str,
    ) -> AppResult<()> {
        PgCustomerRepository::record_exchange_in_tx(tx, branch, customer_id, details, document_ref)
            .await
    }

    pub async fn reveal(
        &self,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        let org_id = self.repo.organization_of_branch(branch).await?;

        let target = Target::new(&org_id.to_string(), Some(branch.clone()), None);
        let grant = Authorizer::check(Some(actor), Permission::IdentityDocumentRead, &target, now)
            .map_err(|_| AppError::Forbidden)?;

        let row: Option<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT encrypted_value, key_id FROM customer_identities \
             WHERE organization_id = $1 AND customer_id = $2 AND type = 'PASSPORT' AND status = 'ACTIVE' \
             LIMIT 1",
        )
        .bind(org_id)
        .bind(customer_id)
        .fetch_optional(self.repo.pool())
        .await
        .map_err(AppError::Database)?;

        let (encrypted_value, key_id) = row.ok_or_else(|| AppError::NotFound {
            entity: "customer_identity",
            id: customer_id.to_string(),
        })?;

        let keys = get_pii_keys()?;
        let cipher = PiiCipher::new(keys);
        let aad = format!("{customer_id}PASSPORT");
        let sealed = Sealed {
            key_id,
            bytes: encrypted_value,
        };
        let decrypted = cipher
            .decrypt(aad.as_bytes(), &sealed)
            .map_err(|e| AppError::Internal(format!("decrypt failed: {e}")))?;

        let val: serde_json::Value = serde_json::from_slice(&decrypted)
            .map_err(|e| AppError::Internal(format!("invalid JSON payload: {e}")))?;

        let passport_number = val
            .get("number")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Internal("missing passport number in decrypted payload".to_string())
            })?
            .to_string();

        let role_str = grant
            .roles_used
            .first()
            .map(|r| r.as_str())
            .unwrap_or("TELLER");

        sqlx::query(
            "INSERT INTO audit_events ( \
                 actor_id, actor_role, scope, branch_id, action, \
                 object_ref, before_redacted, after_redacted, at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&actor.user_id)
        .bind(role_str)
        .bind("BRANCH")
        .bind(branch.as_str())
        .bind("identityReveal")
        .bind(customer_id)
        .bind(Option::<serde_json::Value>::None)
        .bind(Some(serde_json::json!({ "reason": reason })))
        .bind(now)
        .execute(self.repo.pool())
        .await
        .map_err(AppError::Database)?;

        Ok(passport_number)
    }
}

/// TX-4 #672 Q1 = B: what create needs of a customer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeCustomer {
    pub id: String,
    pub mode: Mode,
    pub display_name: String,
    pub nationality: String,
}
