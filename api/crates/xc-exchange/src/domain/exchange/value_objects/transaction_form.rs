//! TransactionForm value object and validation.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::input;
use xc_kernel::money::{CompatRate, Money, NumberError, Quote, Rate, Side};

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::domain::exchange::commands::save_transaction::SaveTransaction;
use crate::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;

pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_DATE_FORMAT: &str = "รูปแบบวันที่ไม่ถูกต้อง";
pub const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
pub const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
pub const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
pub const INCOMPLETE: &str = "ข้อมูลรายการไม่ครบ";
pub const NO_PAYMENT_REF: &str = "ต้องระบุเลขอ้างอิงการโอน";
pub const TOO_LARGE: &str = "ยอดเงินสูงเกินขอบเขตระบบ";
pub const FEE_ABOVE_PRINCIPAL: &str = "ค่าบริการสูงกว่ายอดซื้อ";
pub const FILE_MISSING: &str = "แนบเอกสารไม่ครบ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionForm {
    pub customer: CustomerDetails,
    pub submission_id: String,
    pub customer_id: String,
    pub side: Side,
    pub currency: String,
    pub amount: Money,
    pub rate: Rate,
    pub client_rate: Decimal,
    pub fee: Money,
    pub bank_fee: Money,
    pub method: PaymentMethod,
    pub payment_ref: String,
    pub account: String,
    pub staff: String,
    pub booking: String,
    pub notes: String,
    pub value_date: NaiveDate,
    pub paid_at: DateTime<Utc>,
    pub passport: String,
    pub slip: Option<String>,
    pub reference_rate: Option<Rate>,
    pub client_reference_rate: Option<Decimal>,
    pub quote: Quote,
}

impl TransactionForm {
    /// TX-4 #672 G5: the steps' form rebuilt from a stored exchange (design §4.5). Every figure is
    /// the stored one: the first line's (TX-1 keeps one line, §9.1) and the payment's. The stored
    /// `rate_snapshot` and `reference_rate` are per `unit_size` (RE-4c #757); the form's are per 1.
    /// The exchange keeps no plain passport (#554 K4), so `passport_no` and the dates stay empty.
    pub fn from_transaction(
        t: &crate::domain::transaction::transaction::ExchangeTransaction,
    ) -> AppResult<TransactionForm> {
        let line = &t.lines[0];
        let unit = line.unit_size.value();
        let client_rate = line.rate_snapshot.value() / unit;
        let client_reference_rate = line.reference_rate.map(|r| r.value() / unit);
        let quote = match line.shop_side {
            Side::Sell => Quote {
                principal: line.thb_amount,
                received: line.settle_thb,
                paid: Money::ZERO,
            },
            Side::Buy => Quote {
                principal: line.thb_amount,
                received: Money::ZERO,
                paid: line.settle_thb,
            },
        };
        Ok(TransactionForm {
            customer: CustomerDetails {
                name: t.customer_name.clone(),
                passport_no: String::new(),
                nationality: t.nationality.clone(),
                birth_date: None,
                passport_expiry: None,
                mode: t.mode,
            },
            submission_id: t.submission_id.clone(),
            customer_id: t.customer_id.clone(),
            side: line.shop_side,
            currency: line.currency.as_str().to_owned(),
            amount: money_of(t.id.0, line.quantity.value())?,
            rate: per_one_rate(t.id.0, client_rate)?,
            client_rate,
            fee: line.fee_thb,
            bank_fee: t.payment.bank_fee_thb,
            method: t.payment.method,
            payment_ref: t.payment.reference.clone(),
            account: t.payment.account.clone(),
            staff: t.staff_id.clone(),
            booking: t.booking_no.clone(),
            notes: t.notes.clone(),
            value_date: t.value_date,
            paid_at: t.payment.paid_at,
            passport: t.passport_file_key.clone(),
            slip: t.payment.slip_file_key.clone(),
            reference_rate: client_reference_rate
                .map(|r| per_one_rate(t.id.0, r))
                .transpose()?,
            client_reference_rate,
            quote,
        })
    }

    pub fn parse(cmd: &SaveTransaction) -> AppResult<Self> {
        // Step 1: Customer details
        let customer = CustomerDetails::parse(&cmd.customer)?;

        // Step 2: Submission ID
        let submission_id = input::text(&cmd.submission_id, 60, true)?;

        // Step 3: Customer ID
        let customer_id = input::text(&cmd.customer_id, 40, true)?;

        // Step 4: Side / Type
        let side_str = input::text(&cmd.side, 4, true)?;

        // Step 5: Currency
        let currency_str = input::text(&cmd.currency, 3, true)?;

        // Step 6: Amount
        let amount = Money::parse(&cmd.amount).map_err(input::number_error)?;
        if amount.is_zero() {
            return Err(AppError::Domain(BAD_NUMBER.to_owned()));
        }

        // Step 7: Rate
        let client_rate = CompatRate::parse_per_one(&cmd.rate).map_err(rate_refusal)?;
        let rate = form_rate(&cmd.rate, client_rate).map_err(rate_refusal)?;

        // Step 8: Fee
        let fee = Money::parse(&cmd.fee).map_err(input::number_error)?;

        // Step 9: Bank Fee
        let bank_fee = Money::parse(&cmd.bank_fee).map_err(input::number_error)?;

        // Step 10: Method
        let method_str = input::text(&cmd.method, 30, true)?;

        // Step 11: Payment Ref
        let payment_ref = input::text(&cmd.payment_ref, 80, false)?;

        // Step 12: Account
        let account = input::text(&cmd.account, 100, true)?;

        // Step 13: Staff
        let staff = input::text(&cmd.staff, 40, true)?;

        // Step 14: Booking
        let booking = input::text(&cmd.booking, 60, true)?;

        // Step 15: Notes
        let notes = input::text(&cmd.notes, 1000, false)?;

        // Step 16: Value Date
        let value_date = input::calendar_date(input::text(&cmd.value_date, 10, true)?)?;

        // Step 17: Paid At
        let paid_at_str = input::text(&cmd.paid_at, 30, true)?;

        // Step 18: Incomplete checks (submission_id, side, currency, method, paid_at, confirmations)
        if submission_id.len() < 16
            || !submission_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(AppError::Domain(INCOMPLETE.to_owned()));
        }

        let side = match side_str {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => return Err(AppError::Domain(INCOMPLETE.to_owned())),
        };

        if currency_str.len() != 3 || !currency_str.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(AppError::Domain(INCOMPLETE.to_owned()));
        }

        let method = match method_str {
            "Cash" => PaymentMethod::Cash,
            "SCB Transfer" => PaymentMethod::ScbTransfer,
            "Bank Transfer" => PaymentMethod::BankTransfer,
            "Credit Card" => PaymentMethod::CreditCard,
            "Alipay" => PaymentMethod::Alipay,
            "PromptPay" => PaymentMethod::PromptPay,
            _ => return Err(AppError::Domain(INCOMPLETE.to_owned())),
        };

        let Ok(paid_at_parsed) = DateTime::parse_from_rfc3339(paid_at_str) else {
            return Err(AppError::Domain(INCOMPLETE.to_owned()));
        };
        let paid_at = paid_at_parsed.with_timezone(&Utc);

        if !cmd.payment_confirmed || !cmd.identity_confirmed {
            return Err(AppError::Domain(INCOMPLETE.to_owned()));
        }

        // Step 19: Payment Ref required if non-cash
        if !method.is_cash() && payment_ref.is_empty() {
            return Err(AppError::Domain(NO_PAYMENT_REF.to_owned()));
        }

        // Step 20: Principal limit check
        let amount_dec = Decimal::from_i128_with_scale(amount.minor_i128(), 2);
        let raw_principal = amount_dec * client_rate;
        let mut principal_dec =
            raw_principal.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        principal_dec.rescale(2);
        let principal_minor = principal_dec.mantissa();
        if principal_minor > 9_007_199_254_740_991 {
            return Err(AppError::Domain(TOO_LARGE.to_owned()));
        }
        let principal = Money::try_from_minor_i128(principal_minor)
            .map_err(|_| AppError::Domain(BAD_NUMBER.to_owned()))?;

        // Step 21: Fee exceeds principal check
        let quote = match side {
            Side::Buy => {
                if fee > principal {
                    return Err(AppError::Domain(FEE_ABOVE_PRINCIPAL.to_owned()));
                }
                let paid = Money::try_from_minor_i128(principal.minor_i128() - fee.minor_i128())
                    .map_err(|_| AppError::Domain(BAD_NUMBER.to_owned()))?;
                Quote {
                    principal,
                    received: Money::ZERO,
                    paid,
                }
            }
            Side::Sell => {
                let received =
                    Money::try_from_minor_i128(principal.minor_i128() + fee.minor_i128())
                        .map_err(|_| AppError::Domain(BAD_NUMBER.to_owned()))?;
                Quote {
                    principal,
                    received,
                    paid: Money::ZERO,
                }
            }
        };

        // Step 22: Passport required
        let passport = match &cmd.passport {
            Some(p) => p.clone(),
            None => return Err(AppError::Domain(FILE_MISSING.to_owned())),
        };

        // Step 23: Slip required if non-cash
        if !method.is_cash() && cmd.slip.is_none() {
            return Err(AppError::Domain(FILE_MISSING.to_owned()));
        }
        let slip = cmd.slip.clone();

        // Step 24: Reference rate
        let (reference_rate, client_reference_rate) = match cmd.reference_rate.as_deref() {
            None | Some("") => (None, None),
            Some(s) => {
                let client_ref = CompatRate::parse_per_one(s).map_err(rate_refusal)?;
                let ref_rate = form_rate(s, client_ref).map_err(rate_refusal)?;
                (Some(ref_rate), Some(client_ref))
            }
        };

        Ok(Self {
            customer,
            submission_id: submission_id.to_owned(),
            customer_id: customer_id.to_owned(),
            side,
            currency: currency_str.to_owned(),
            amount,
            rate,
            client_rate,
            fee,
            bank_fee,
            method,
            payment_ref: payment_ref.to_owned(),
            account: account.to_owned(),
            staff: staff.to_owned(),
            booking: booking.to_owned(),
            notes: notes.to_owned(),
            value_date,
            paid_at,
            passport,
            slip,
            reference_rate,
            client_reference_rate,
            quote,
        })
    }
}

fn form_rate(text: &str, per_one: Decimal) -> Result<Rate, NumberError> {
    Rate::parse(text).or_else(|_| Rate::parse(&format!("{:.8}", per_one)))
}

/// A stored exact per-1 rate as the form's `Rate`, as `parse` falls back to it: at most 8 places.
fn per_one_rate(id: uuid::Uuid, per_one: Decimal) -> AppResult<Rate> {
    Rate::parse(&per_one.normalize().to_string())
        .or_else(|_| Rate::parse(&format!("{:.8}", per_one)))
        .map_err(|_| {
            AppError::Internal(format!(
                "transaction {id} stored per-1 rate {per_one} is out of range for TransactionForm"
            ))
        })
}

/// A stored quantity as the form's amount (two places, half up, as `parse` reads it).
fn money_of(id: uuid::Uuid, value: Decimal) -> AppResult<Money> {
    let rounded = value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    Money::parse(&rounded.normalize().to_string()).map_err(|_| {
        AppError::Internal(format!(
            "transaction {id} stored quantity {value} is out of range for TransactionForm"
        ))
    })
}

/// The form's sentence for a per-1 rate `CompatRate::parse_per_one` refuses (#770).
fn rate_refusal(e: NumberError) -> AppError {
    match e {
        NumberError::Invalid => AppError::Domain(BAD_NUMBER.to_owned()),
        NumberError::TooManyDecimals => AppError::Domain(TOO_MANY_DECIMALS.to_owned()),
    }
}
