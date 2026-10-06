//! TransactionForm value object and validation.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::money::{Money, NumberError, Quote, QuoteError, Rate, Side};

use crate::domain::customer::value_objects::customer_details::CustomerDetails;
use crate::domain::exchange::commands::save_transaction::SaveTransaction;
use crate::domain::exchange::value_objects::payment_method::PaymentMethod;

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
    pub quote: Quote,
}

impl TransactionForm {
    pub fn parse(cmd: &SaveTransaction) -> AppResult<Self> {
        // Step 1: Customer details
        let customer = CustomerDetails::parse(&cmd.customer)?;

        // Step 2: Submission ID
        let submission_id = validate_text(&cmd.submission_id, 60, true)?;

        // Step 3: Customer ID
        let customer_id = validate_text(&cmd.customer_id, 40, true)?;

        // Step 4: Side / Type
        let side_str = validate_text(&cmd.side, 4, true)?;

        // Step 5: Currency
        let currency_str = validate_text(&cmd.currency, 3, true)?;

        // Step 6: Amount
        let amount = Money::parse(&cmd.amount).map_err(map_number_err)?;
        if amount.is_zero() {
            return Err(AppError::Domain(BAD_NUMBER.to_owned()));
        }

        // Step 7: Rate
        let rate = Rate::parse(&cmd.rate).map_err(map_number_err)?;

        // Step 8: Fee
        let fee = Money::parse(&cmd.fee).map_err(map_number_err)?;

        // Step 9: Bank Fee
        let bank_fee = Money::parse(&cmd.bank_fee).map_err(map_number_err)?;

        // Step 10: Method
        let method_str = validate_text(&cmd.method, 30, true)?;

        // Step 11: Payment Ref
        let payment_ref = validate_text(&cmd.payment_ref, 80, false)?;

        // Step 12: Account
        let account = validate_text(&cmd.account, 100, true)?;

        // Step 13: Staff
        let staff = validate_text(&cmd.staff, 40, true)?;

        // Step 14: Booking
        let booking = validate_text(&cmd.booking, 60, true)?;

        // Step 15: Notes
        let notes = validate_text(&cmd.notes, 1000, false)?;

        // Step 16: Value Date
        let value_date = parse_value_date(&cmd.value_date)?;

        // Step 17: Paid At
        let paid_at_str = validate_text(&cmd.paid_at, 30, true)?;

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
        let quote_zero = Quote::compute(side, amount, rate, Money::ZERO)
            .map_err(|_| AppError::Domain(BAD_NUMBER.to_owned()))?;
        if quote_zero.principal.minor_i128() > 9_007_199_254_740_991 {
            return Err(AppError::Domain(TOO_LARGE.to_owned()));
        }

        // Step 21: Fee exceeds principal check
        let quote = match Quote::compute(side, amount, rate, fee) {
            Ok(q) => q,
            Err(QuoteError::FeeExceedsPrincipal) => {
                return Err(AppError::Domain(FEE_ABOVE_PRINCIPAL.to_owned()));
            }
            Err(QuoteError::ZeroAmount) => return Err(AppError::Domain(BAD_NUMBER.to_owned())),
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
        let reference_rate = match cmd.reference_rate.as_deref() {
            None | Some("") => None,
            Some(s) => Some(Rate::parse(s).map_err(map_number_err)?),
        };

        Ok(Self {
            customer,
            submission_id: submission_id.to_owned(),
            customer_id: customer_id.to_owned(),
            side,
            currency: currency_str.to_owned(),
            amount,
            rate,
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
            quote,
        })
    }
}

fn validate_text(raw: &str, max_utf16: usize, required: bool) -> AppResult<&str> {
    let trimmed = raw.trim();
    let count = trimmed.encode_utf16().count();
    if (required && count == 0) || count > max_utf16 {
        return Err(AppError::Domain(BAD_TEXT.to_owned()));
    }
    Ok(trimmed)
}

fn map_number_err(err: NumberError) -> AppError {
    match err {
        NumberError::TooManyDecimals => AppError::Domain(TOO_MANY_DECIMALS.to_owned()),
        NumberError::Invalid => AppError::Domain(BAD_NUMBER.to_owned()),
    }
}

fn parse_value_date(raw: &str) -> AppResult<NaiveDate> {
    let trimmed = validate_text(raw, 10, true)?;
    let bytes = trimmed.as_bytes();
    if bytes.len() != 10
        || !bytes[0..4].iter().all(u8::is_ascii_digit)
        || bytes[4] != b'-'
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || bytes[7] != b'-'
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return Err(AppError::Domain(BAD_DATE_FORMAT.to_owned()));
    }

    let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|_| AppError::Domain(BAD_DATE.to_owned()))?;

    let floor = NaiveDate::from_ymd_opt(1920, 4, 2).expect("valid date");
    if date < floor {
        return Err(AppError::Domain(BAD_DATE.to_owned()));
    }

    Ok(date)
}
