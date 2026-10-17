//! `BookingService` (design §4.3, §4.4, §5): creates, confirms, cancels,
//! reschedules and reissues QR for bookings.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::booking::booking::Booking;
use crate::domain::booking::booking_error::BookingError;
use crate::domain::booking::booking_event::BookingEventKind;
use crate::domain::booking::booking_machine::BookingMachine;
use crate::domain::booking::booking_status::BookingStatus;
use crate::domain::booking::qr_token::QrToken;
use crate::domain::booking::repository::BookingRepository;
use crate::domain::booking::source_channel::SourceChannel;
use crate::domain::policy::booking_policy::BookingPolicy;
use crate::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use crate::domain::slot::repository::SlotRepository;
use xc_customer::application::customer::contact_reserve::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kernel::pii::IdentityType;
use xc_referral::application::capture_service::ReferralCaptureService;
use xc_referral::domain::referral_code::ReferralCode;

#[derive(Clone, Debug)]
pub struct CreateBookingInput {
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub slot_id: Uuid,
    pub shop_side: Side,
    pub requested_currency: String,
    pub requested_quantity: Decimal,
    pub channel: SourceChannel,
    pub actor_id: String,
    pub customer_id: Option<String>,
    pub contact: Option<ContactInput>,
    pub quote_id: Option<Uuid>,
    pub referral_code: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ContactInput {
    pub display_name: String,
    pub phone: String,
}

#[derive(Clone, Debug)]
pub struct CreatedBooking {
    pub booking: Booking,
    pub qr_token: String,
}

#[derive(Clone, Debug)]
pub struct ReissuedQr {
    pub booking: Booking,
    pub qr_token: String,
}

#[derive(sqlx::FromRow)]
struct SlotQueryRow {
    service_date: NaiveDate,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    capacity: i32,
    reserved: i32,
    status: String,
}

#[derive(sqlx::FromRow)]
struct RescheduleSlotQueryRow {
    branch_id: String,
    service_date: NaiveDate,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    capacity: i32,
    reserved: i32,
    status: String,
}

pub struct BookingService {
    pool: PgPool,
    bookings: Arc<dyn BookingRepository>,
    slots: Arc<dyn SlotRepository>,
    contact_reserve: Arc<ContactReserve>,
    referral_capture: Option<Arc<ReferralCaptureService>>,
    pii_keys: xc_kernel::pii::PiiKeys,
    queue: Option<Arc<dyn xc_kernel::queue_tickets::QueueTickets>>,
}

impl BookingService {
    pub fn new(
        pool: PgPool,
        bookings: Arc<dyn BookingRepository>,
        slots: Arc<dyn SlotRepository>,
        contact_reserve: Arc<ContactReserve>,
        referral_capture: Option<Arc<ReferralCaptureService>>,
        pii_keys: xc_kernel::pii::PiiKeys,
    ) -> Self {
        Self {
            pool,
            bookings,
            slots,
            contact_reserve,
            referral_capture,
            pii_keys,
            queue: None,
        }
    }

    pub fn with_queue(mut self, queue: Arc<dyn xc_kernel::queue_tickets::QueueTickets>) -> Self {
        self.queue = Some(queue);
        self
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn create_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        input: CreateBookingInput,
        now: DateTime<Utc>,
    ) -> Result<CreatedBooking, BookingError> {
        // 1. Branch check
        let booking_enabled: Option<bool> = sqlx::query_scalar(
            "SELECT booking_enabled FROM branches WHERE id = $1 AND organization_id = $2",
        )
        .bind(input.branch_id.as_str())
        .bind(input.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        match booking_enabled {
            Some(true) => {}
            _ => return Err(BookingError::BookingsClosed),
        }

        // 2. Slot rule check
        let rule_exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM booking_slot_rules WHERE branch_id = $1 AND organization_id = $2",
        )
        .bind(input.branch_id.as_str())
        .bind(input.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        if rule_exists.is_none() {
            return Err(BookingError::BookingsClosed);
        }

        // 3. Booking policies check
        let policy_row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
             FROM booking_policies WHERE organization_id = $1",
        )
        .bind(input.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let (cutoff, grace, expiry) = match policy_row {
            Some(p) => p,
            None => return Err(BookingError::BookingsClosed),
        };

        let policy = BookingPolicy {
            cancel_cutoff_minutes: cutoff,
            no_show_grace_minutes: grace,
            confirm_expiry_minutes: expiry,
        };

        // 4. Slot check
        let slot_row: Option<SlotQueryRow> = sqlx::query_as(
            "SELECT service_date, start_at, end_at, capacity, reserved, status \
             FROM booking_slots \
             WHERE id = $1 AND organization_id = $2 AND branch_id = $3",
        )
        .bind(input.slot_id)
        .bind(input.organization_id)
        .bind(input.branch_id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let slot_row = match slot_row {
            Some(s) => s,
            None => return Err(BookingError::SlotUnavailable),
        };

        if slot_row.status != "OPEN" || slot_row.start_at <= now {
            return Err(BookingError::SlotUnavailable);
        }

        let slot = BookingSlot {
            id: input.slot_id,
            organization_id: input.organization_id,
            branch_id: input.branch_id.clone(),
            service_date: slot_row.service_date,
            start_at: slot_row.start_at,
            end_at: slot_row.end_at,
            capacity: slot_row.capacity,
            reserved: slot_row.reserved,
            status: SlotStatus::Open,
        };

        // 5. Quote validation
        if let Some(quote_id) = input.quote_id {
            let quote_row: Option<(String, String, String, String, String, Decimal)> = sqlx::query_as(
                "SELECT q.branch_id, q.status, q.channel, ql.shop_side, ql.currency, ql.quantity \
                 FROM quotes q \
                 JOIN quote_lines ql ON ql.quote_id = q.id \
                 WHERE q.id = $1 AND q.organization_id = $2",
            )
            .bind(quote_id)
            .bind(input.organization_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let (q_branch, q_status, q_channel, q_side, q_curr, q_qty) = match quote_row {
                Some(r) => r,
                None => return Err(BookingError::ValidationFailed { field: "quoteId" }),
            };

            let expected_side = match input.shop_side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };

            if q_branch != input.branch_id.as_str()
                || q_status == "CANCELLED"
                || q_channel != "BOOKING"
                || q_side != expected_side
                || q_curr != input.requested_currency
                || q_qty != input.requested_quantity
            {
                return Err(BookingError::ValidationFailed { field: "quoteId" });
            }
        }

        // 6. Referral code pre-validation
        if input.channel == SourceChannel::Partner && input.referral_code.is_none() {
            return Err(BookingError::ValidationFailed {
                field: "referralCode",
            });
        }
        let parsed_referral_code = if let Some(ref code_str) = input.referral_code {
            match ReferralCode::parse(code_str) {
                Ok(code) => Some(code),
                Err(_) => {
                    return Err(BookingError::ValidationFailed {
                        field: "referralCode",
                    });
                }
            }
        } else {
            None
        };

        // Customer resolution
        let customer_id = if let Some(ref cid) = input.customer_id {
            let exists: Option<bool> = sqlx::query_scalar(
                "SELECT true FROM customers WHERE id = $1 AND organization_id = $2",
            )
            .bind(cid)
            .bind(input.organization_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            if exists.is_none() {
                return Err(BookingError::NotFound);
            }
            cid.clone()
        } else if let Some(ref contact) = input.contact {
            if xc_kernel::pii::normalize(IdentityType::Phone, &contact.phone, None).is_err() {
                return Err(BookingError::ValidationFailed { field: "phone" });
            }
            let res = self
                .contact_reserve
                .reserve_in(
                    tx,
                    input.organization_id,
                    &contact.display_name,
                    &contact.phone,
                    now,
                )
                .await
                .map_err(BookingError::Store)?;
            res.customer_id
        } else {
            return Err(BookingError::ValidationFailed {
                field: "customerId",
            });
        };

        // 8. Reserve slot place atomically
        self.slots
            .reserve_in(tx, input.slot_id)
            .await
            .map_err(BookingError::from)?;

        // 9. Initial status and expiry
        let initial_status = input.channel.start_status();
        let confirm_expires_at = if initial_status == BookingStatus::PendingCustomerConfirmation {
            Some(policy.confirm_expires_at(now, &slot))
        } else {
            None
        };

        // 10. Generate booking id, booking no, qr token
        let booking_uuid = Uuid::new_v4();
        let booking_id = format!("BK-{booking_uuid}");
        let booking_no = self
            .bookings
            .next_booking_no(tx, input.organization_id, slot.service_date)
            .await?;

        let qr_key = QrToken::key_from_pii(&self.pii_keys);
        let qr_version = 1;
        let qr_token = QrToken::derive(&qr_key, &booking_uuid, qr_version);
        let qr_token_hash = QrToken::hash(&qr_token);

        let booking = Booking {
            id: booking_id,
            organization_id: input.organization_id,
            branch_id: input.branch_id.clone(),
            booking_no,
            customer_id,
            slot_id: input.slot_id,
            quote_id: input.quote_id,
            shop_side: input.shop_side,
            requested_currency: input.requested_currency,
            requested_quantity: input.requested_quantity,
            source_channel: input.channel,
            status: initial_status,
            qr_token_hash,
            qr_version,
            confirm_expires_at,
            checked_in_at: None,
            transaction_id: None,
            partner_attribution_id: None,
            ended_reason: None,
            created_by: input.actor_id.clone(),
            created_at: now,
            updated_at: now,
        };

        self.bookings.insert_in(tx, &booking).await?;

        // 11. Referral capture
        if let Some(code) = parsed_referral_code {
            if let Some(ref referral_capture) = self.referral_capture {
                let capture_input = xc_referral::application::capture_service::CaptureInput {
                    code,
                    on: xc_referral::domain::capture::CaptureTarget::Booking(booking.id.clone()),
                    source: match input.channel {
                        SourceChannel::App => xc_referral::domain::capture::CaptureSource::App,
                        SourceChannel::Desk => xc_referral::domain::capture::CaptureSource::Desk,
                        SourceChannel::Partner => {
                            xc_referral::domain::capture::CaptureSource::Booking
                        }
                    },
                    customer_sub: None,
                    by: input.actor_id.clone(),
                    at: now,
                };
                referral_capture
                    .capture_in(tx, capture_input)
                    .await
                    .map_err(|e| match e {
                        xc_referral::application::capture_service::CaptureError::BadCode => {
                            BookingError::ValidationFailed {
                                field: "referralCode",
                            }
                        }
                        xc_referral::application::capture_service::CaptureError::Store(err) => {
                            BookingError::Store(err)
                        }
                        _ => BookingError::ValidationFailed {
                            field: "referralCode",
                        },
                    })?;
            }
        }

        Ok(CreatedBooking { booking, qr_token })
    }

    pub async fn create(
        &self,
        input: CreateBookingInput,
        now: DateTime<Utc>,
    ) -> Result<CreatedBooking, BookingError> {
        let request_payload = serde_json::json!({
            "organization_id": input.organization_id,
            "branch_id": input.branch_id.as_str(),
            "slot_id": input.slot_id,
            "shop_side": match input.shop_side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            },
            "requested_currency": input.requested_currency,
            "requested_quantity": input.requested_quantity.to_string(),
            "channel": input.channel.as_str(),
            "customer_id": input.customer_id,
            "contact": input.contact.as_ref().map(|c| (&c.display_name, &c.phone)),
            "quote_id": input.quote_id,
            "referral_code": input.referral_code,
        });

        let request_hash = {
            let mut hasher = Sha256::new();
            hasher.update(request_payload.to_string().as_bytes());
            format!("{:x}", hasher.finalize())
        };

        if let Some(ref key) = input.idempotency_key {
            let existing_record: Option<(String, Option<serde_json::Value>, String)> =
                sqlx::query_as(
                    "SELECT request_hash, response_ref, status \
                     FROM idempotency_records \
                     WHERE organization_id = $1 AND operation = 'bookingCreate' AND key = $2",
                )
                .bind(input.organization_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(pos_error::AppError::Database)?;

            if let Some((stored_hash, response_ref, _status)) = existing_record {
                if stored_hash != request_hash {
                    return Err(BookingError::IdempotencyConflict);
                }
                let resp = response_ref.ok_or_else(|| {
                    BookingError::Store(pos_error::AppError::Domain(
                        "idempotency record missing response_ref".to_string(),
                    ))
                })?;
                let booking_id = resp["booking_id"].as_str().ok_or_else(|| {
                    BookingError::Store(pos_error::AppError::Domain(
                        "response_ref missing booking_id".to_string(),
                    ))
                })?;

                let mut tx = self
                    .pool
                    .begin()
                    .await
                    .map_err(pos_error::AppError::Database)?;
                let booking = self
                    .bookings
                    .by_id(&mut tx, booking_id)
                    .await?
                    .ok_or(BookingError::NotFound)?;
                tx.rollback().await.map_err(pos_error::AppError::Database)?;

                let booking_uuid =
                    Uuid::parse_str(booking.id.strip_prefix("BK-").unwrap_or(&booking.id))
                        .map_err(|e| {
                            pos_error::AppError::Domain(format!("invalid booking uuid: {e}"))
                        })?;
                let qr_key = QrToken::key_from_pii(&self.pii_keys);
                let qr_token = QrToken::derive(&qr_key, &booking_uuid, booking.qr_version);

                return Ok(CreatedBooking { booking, qr_token });
            }
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;

        BranchTx::lock_also(&mut tx, &input.branch_id).await?;

        let created = self.create_in(&mut tx, input.clone(), now).await?;

        // 12. Idempotency record (never storing plain QR token!)
        if let Some(ref key) = input.idempotency_key {
            let response_ref = serde_json::json!({
                "booking_id": created.booking.id,
            });
            sqlx::query(
                "INSERT INTO idempotency_records ( \
                     organization_id, operation, key, request_hash, response_ref, status, created_at, claim_token \
                 ) VALUES ($1, 'bookingCreate', $2, $3, $4, 'COMPLETED', $5, NULL)",
            )
            .bind(input.organization_id)
            .bind(key)
            .bind(&request_hash)
            .bind(response_ref)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;
        }

        tx.commit().await.map_err(pos_error::AppError::Database)?;

        Ok(created)
    }

    pub async fn confirm_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let booking = self
            .bookings
            .by_id(tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        let policy_row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
             FROM booking_policies WHERE organization_id = $1",
        )
        .bind(booking.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let (cutoff, grace, expiry) = policy_row.ok_or(BookingError::BookingsClosed)?;
        let policy = BookingPolicy {
            cancel_cutoff_minutes: cutoff,
            no_show_grace_minutes: grace,
            confirm_expiry_minutes: expiry,
        };

        BookingMachine::confirm(&booking, now, &policy)?;

        let updated = self
            .bookings
            .transition_in(
                tx,
                booking_id,
                BookingStatus::PendingCustomerConfirmation,
                BookingStatus::Confirmed,
                actor,
                BookingEventKind::Confirm,
                None,
                None,
                now,
            )
            .await?;

        Ok(updated)
    }

    pub async fn confirm(
        &self,
        booking_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;
        let booking = self
            .bookings
            .by_id(&mut tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

        let updated = self.confirm_in(&mut tx, booking_id, actor, now).await?;

        tx.commit().await.map_err(pos_error::AppError::Database)?;
        Ok(updated)
    }

    pub async fn cancel_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking_id: &str,
        actor: &str,
        is_staff: bool,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let booking = self
            .bookings
            .by_id(tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        let policy_row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
             FROM booking_policies WHERE organization_id = $1",
        )
        .bind(booking.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let (cutoff, grace, expiry) = policy_row.ok_or(BookingError::BookingsClosed)?;
        let policy = BookingPolicy {
            cancel_cutoff_minutes: cutoff,
            no_show_grace_minutes: grace,
            confirm_expiry_minutes: expiry,
        };

        let slot_row: Option<SlotQueryRow> = sqlx::query_as(
            "SELECT service_date, start_at, end_at, capacity, reserved, status \
             FROM booking_slots WHERE id = $1",
        )
        .bind(booking.slot_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let slot_row = slot_row.ok_or(BookingError::NotFound)?;

        let slot = BookingSlot {
            id: booking.slot_id,
            organization_id: booking.organization_id,
            branch_id: booking.branch_id.clone(),
            service_date: slot_row.service_date,
            start_at: slot_row.start_at,
            end_at: slot_row.end_at,
            capacity: slot_row.capacity,
            reserved: slot_row.reserved,
            status: match slot_row.status.as_str() {
                "CLOSED" => SlotStatus::Closed,
                _ => SlotStatus::Open,
            },
        };

        BookingMachine::cancel(&booking, actor, is_staff, reason, now, &policy, &slot)?;

        if booking.status == BookingStatus::CheckedIn {
            if let Some(queue) = &self.queue {
                queue
                    .cancel_active_for_booking_in(tx, &booking.branch_id, booking_id, now)
                    .await?;
            }
        }

        if booking.status.holds_place() {
            self.slots
                .release_in(tx, booking.slot_id)
                .await
                .map_err(BookingError::from)?;
        }

        let updated = self
            .bookings
            .transition_in(
                tx,
                booking_id,
                booking.status,
                BookingStatus::Cancelled,
                actor,
                BookingEventKind::Cancel,
                reason,
                None,
                now,
            )
            .await?;

        Ok(updated)
    }

    pub async fn cancel(
        &self,
        booking_id: &str,
        actor: &str,
        is_staff: bool,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;
        let booking = self
            .bookings
            .by_id(&mut tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

        let updated = self
            .cancel_in(&mut tx, booking_id, actor, is_staff, reason, now)
            .await?;

        tx.commit().await.map_err(pos_error::AppError::Database)?;
        Ok(updated)
    }

    pub async fn reschedule_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking_id: &str,
        new_slot_id: Uuid,
        actor: &str,
        is_staff: bool,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let booking = self
            .bookings
            .by_id(tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        if booking.slot_id == new_slot_id {
            return Ok(booking);
        }

        let policy_row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
             FROM booking_policies WHERE organization_id = $1",
        )
        .bind(booking.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let (cutoff, grace, expiry) = policy_row.ok_or(BookingError::BookingsClosed)?;
        let policy = BookingPolicy {
            cancel_cutoff_minutes: cutoff,
            no_show_grace_minutes: grace,
            confirm_expiry_minutes: expiry,
        };

        let current_slot_row: Option<SlotQueryRow> = sqlx::query_as(
            "SELECT service_date, start_at, end_at, capacity, reserved, status \
             FROM booking_slots WHERE id = $1",
        )
        .bind(booking.slot_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let current_slot_row = current_slot_row.ok_or(BookingError::NotFound)?;

        let current_slot = BookingSlot {
            id: booking.slot_id,
            organization_id: booking.organization_id,
            branch_id: booking.branch_id.clone(),
            service_date: current_slot_row.service_date,
            start_at: current_slot_row.start_at,
            end_at: current_slot_row.end_at,
            capacity: current_slot_row.capacity,
            reserved: current_slot_row.reserved,
            status: match current_slot_row.status.as_str() {
                "CLOSED" => SlotStatus::Closed,
                _ => SlotStatus::Open,
            },
        };

        let new_slot_row: Option<RescheduleSlotQueryRow> = sqlx::query_as(
            "SELECT branch_id, service_date, start_at, end_at, capacity, reserved, status \
             FROM booking_slots WHERE id = $1 AND organization_id = $2",
        )
        .bind(new_slot_id)
        .bind(booking.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let new_slot_row = new_slot_row.ok_or(BookingError::SlotUnavailable)?;

        if new_slot_row.branch_id != booking.branch_id.as_str()
            || new_slot_row.status != "OPEN"
            || new_slot_row.start_at <= now
        {
            return Err(BookingError::SlotUnavailable);
        }

        let new_slot = BookingSlot {
            id: new_slot_id,
            organization_id: booking.organization_id,
            branch_id: BranchId::new(new_slot_row.branch_id),
            service_date: new_slot_row.service_date,
            start_at: new_slot_row.start_at,
            end_at: new_slot_row.end_at,
            capacity: new_slot_row.capacity,
            reserved: new_slot_row.reserved,
            status: SlotStatus::Open,
        };

        BookingMachine::reschedule(
            &booking,
            actor,
            is_staff,
            &new_slot,
            now,
            &policy,
            &current_slot,
        )?;

        self.slots
            .move_in(tx, booking.slot_id, new_slot_id)
            .await
            .map_err(BookingError::from)?;

        let updated = self
            .bookings
            .update_slot_in(tx, booking_id, booking.status, new_slot_id, actor, now)
            .await?;

        Ok(updated)
    }

    pub async fn reschedule(
        &self,
        booking_id: &str,
        new_slot_id: Uuid,
        actor: &str,
        is_staff: bool,
        now: DateTime<Utc>,
    ) -> Result<Booking, BookingError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;
        let booking = self
            .bookings
            .by_id(&mut tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        if booking.slot_id == new_slot_id {
            return Ok(booking);
        }

        BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

        let updated = self
            .reschedule_in(&mut tx, booking_id, new_slot_id, actor, is_staff, now)
            .await?;

        tx.commit().await.map_err(pos_error::AppError::Database)?;
        Ok(updated)
    }

    pub async fn reissue_qr_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<ReissuedQr, BookingError> {
        let booking = self
            .bookings
            .by_id(tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        match booking.status {
            BookingStatus::PendingCustomerConfirmation
            | BookingStatus::Confirmed
            | BookingStatus::CheckedIn => {}
            _ => return Err(BookingError::StaleVersion),
        }

        let booking_uuid =
            Uuid::parse_str(booking.id.strip_prefix("BK-").unwrap_or(&booking.id))
                .map_err(|e| pos_error::AppError::Domain(format!("invalid booking uuid: {e}")))?;
        let new_qr_version = booking.qr_version + 1;
        let qr_key = QrToken::key_from_pii(&self.pii_keys);
        let qr_token = QrToken::derive(&qr_key, &booking_uuid, new_qr_version);
        let qr_token_hash = QrToken::hash(&qr_token);

        let updated = self
            .bookings
            .reissue_qr_in(
                tx,
                booking_id,
                booking.status,
                &qr_token_hash,
                new_qr_version,
                actor,
                now,
            )
            .await?;

        Ok(ReissuedQr {
            booking: updated,
            qr_token,
        })
    }

    pub async fn reissue_qr(
        &self,
        booking_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<ReissuedQr, BookingError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;
        let booking = self
            .bookings
            .by_id(&mut tx, booking_id)
            .await?
            .ok_or(BookingError::NotFound)?;

        match booking.status {
            BookingStatus::PendingCustomerConfirmation
            | BookingStatus::Confirmed
            | BookingStatus::CheckedIn => {}
            _ => return Err(BookingError::StaleVersion),
        }

        BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

        let res = self.reissue_qr_in(&mut tx, booking_id, actor, now).await?;

        tx.commit().await.map_err(pos_error::AppError::Database)?;
        Ok(res)
    }
}
