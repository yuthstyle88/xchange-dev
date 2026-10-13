//! `BookingRepository` (design §5): booking persistence operations within a transaction.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::booking::Booking;
use super::booking_error::BookingError;
use super::booking_event::BookingEventKind;
use super::booking_status::BookingStatus;

pub trait BookingRepository: Send + Sync {
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking: &'a Booking,
    ) -> BoxFuture<'a, Result<(), BookingError>>;

    #[allow(clippy::too_many_arguments)]
    fn transition_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_status: BookingStatus,
        actor_id: &'a str,
        event_kind: BookingEventKind,
        reason: Option<&'a str>,
        detail: Option<serde_json::Value>,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>>;

    fn by_qr_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        hash: &'a [u8],
    ) -> BoxFuture<'a, Result<Option<Booking>, BookingError>>;

    fn by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<Booking>, BookingError>>;

    fn next_booking_no<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org_id: Uuid,
        issued_on: chrono::NaiveDate,
    ) -> BoxFuture<'a, Result<String, BookingError>>;

    fn update_slot_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_slot_id: Uuid,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>>;

    #[allow(clippy::too_many_arguments)]
    fn reissue_qr_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_qr_token_hash: &'a [u8],
        new_qr_version: u32,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>>;
}
