use xc_kernel::money::{Money, Quote, Side};

use crate::domain::kiosk::commands::kiosk_command::KioskCommand;
use crate::domain::kiosk::commands::quote_input::QuoteInput;
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::counter_facts::CounterFacts;
use crate::domain::kiosk::value_objects::kiosk_action::KioskAction;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use crate::domain::kiosk::value_objects::kiosk_role::KioskRole;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::payment_method::PaymentMethod;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;
use crate::domain::kiosk::value_objects::save_attempt::SaveAttempt;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;
use crate::domain::kiosk::value_objects::transition::Transition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KioskMachine;

impl KioskMachine {
    pub fn authorize(action: KioskAction, actor: &KioskActor) -> Result<(), KioskError> {
        match action {
            KioskAction::Rates
            | KioskAction::Quote
            | KioskAction::Confirm
            | KioskAction::Status
            | KioskAction::CustomerReceipt => Ok(()),

            KioskAction::Notifications => match actor {
                KioskActor::Staff { .. } => Ok(()),
                KioskActor::Customer => Err(KioskError::StaffRequired),
            },

            KioskAction::List
            | KioskAction::Accept
            | KioskAction::Get
            | KioskAction::IdentityChecked
            | KioskAction::PublishReceipt => match actor {
                KioskActor::Staff { role, .. } if role.is_desk() => Ok(()),
                _ => Err(KioskError::StaffRequired),
            },

            KioskAction::Manage | KioskAction::Configure => match actor {
                KioskActor::Staff {
                    role: KioskRole::Owner | KioskRole::Rates,
                    ..
                } => Ok(()),
                _ => Err(KioskError::OwnerOrRatesRequired),
            },
        }
    }

    pub fn quote(
        input: &QuoteInput,
        facts: &CounterFacts,
        new_id: &str,
        now: KioskTime,
    ) -> Result<KioskRequest, KioskError> {
        // 1. facts.open is false -> CounterClosed.
        if !facts.open {
            return Err(KioskError::CounterClosed);
        }

        // 2. input.side is None -> DirectionRequired.
        let side = input.side.ok_or(KioskError::DirectionRequired)?;

        // 3. facts.rate is None, or input.currency is None, or input.amount is None,
        // or the amount is zero, or the amount is greater than 10,000,000.00 -> AmountOrCurrencyInvalid.
        let rate = facts.rate.ok_or(KioskError::AmountOrCurrencyInvalid)?;
        let currency = input.currency.ok_or(KioskError::AmountOrCurrencyInvalid)?;
        let amount = input.amount.ok_or(KioskError::AmountOrCurrencyInvalid)?;
        if amount.is_zero() || amount.minor_i128() > 1_000_000_000 {
            return Err(KioskError::AmountOrCurrencyInvalid);
        }

        // 4. Compute
        let (foreign, thb) = match side {
            Side::Buy => {
                let foreign = amount;
                let quote = Quote::compute(Side::Buy, foreign, rate, Money::ZERO)
                    .map_err(|_| KioskError::AmountOrCurrencyInvalid)?;
                (foreign, quote.principal)
            }
            Side::Sell => {
                let a = amount.minor_i128();
                let r = rate.value().mantissa();
                let foreign_hundredths = a * 10_000 / r;
                // 5. foreign is zero -> AmountTooSmall.
                if foreign_hundredths == 0 {
                    return Err(KioskError::AmountTooSmall);
                }
                let foreign_minor: i64 = foreign_hundredths
                    .try_into()
                    .map_err(|_| KioskError::AmountOrCurrencyInvalid)?;
                let foreign = Money::from_minor(foreign_minor)
                    .map_err(|_| KioskError::AmountOrCurrencyInvalid)?;
                let quote = Quote::compute(Side::Sell, foreign, rate, Money::ZERO)
                    .map_err(|_| KioskError::AmountOrCurrencyInvalid)?;
                (foreign, quote.principal)
            }
        };

        // 5. foreign is zero -> AmountTooSmall.
        if foreign.is_zero() {
            return Err(KioskError::AmountTooSmall);
        }

        // 6. The method text is input.payment_method; None and "" both mean "Cash".
        let method_str = match input.payment_method.as_deref() {
            None | Some("") => "Cash",
            Some(s) => s,
        };
        // Side::Buy and the method text is neither "Cash" nor "Bank Transfer" -> BuyPaidInCashOrTransfer.
        if side == Side::Buy && method_str != "Cash" && method_str != "Bank Transfer" {
            return Err(KioskError::BuyPaidInCashOrTransfer);
        }

        // 7. PaymentMethod::parse(method text) is None -> PaymentMethodInvalid.
        let payment_method =
            PaymentMethod::parse(method_str).ok_or(KioskError::PaymentMethodInvalid)?;

        // 8. Ok(KioskRequest { ... })
        Ok(KioskRequest {
            id: new_id.to_string(),
            status: KioskStatus::Quoted,
            created: now,
            staff: None,
            quote: KioskQuote {
                side,
                currency,
                rate,
                foreign,
                thb,
                input_amount: amount,
                payment_method,
            },
            transaction: None,
        })
    }

    pub fn next(
        request: Option<&KioskRequest>,
        command: &KioskCommand,
        actor: &KioskActor,
        facts: &CounterFacts,
        now: KioskTime,
    ) -> Result<Transition, KioskError> {
        match command {
            KioskCommand::Confirm => {
                // 1. request is None -> RequestNotFoundStartAgain.
                let req = request.ok_or(KioskError::RequestNotFoundStartAgain)?;

                // 2. status is Waiting, Accepted, IdChecked or Completed -> unchanged.
                match req.status {
                    KioskStatus::Waiting
                    | KioskStatus::Accepted
                    | KioskStatus::IdChecked
                    | KioskStatus::Completed => {
                        return Ok(Transition {
                            request: req.clone(),
                            event: None,
                        });
                    }
                    _ => {}
                }

                let age = now.epoch_millis() - req.created.epoch_millis();

                // 3. status is not Quoted (that leaves a stored Expired), or age > 300,000 -> QuoteExpired.
                if req.status != KioskStatus::Quoted || age > 300_000 {
                    return Err(KioskError::QuoteExpired);
                }

                // 4. age < 3,000 -> ReviewThreeSeconds.
                if age < 3_000 {
                    return Err(KioskError::ReviewThreeSeconds);
                }

                // 5. facts.open is false, or facts.rate != Some(request.quote.rate) -> RateChangedOrClosed.
                if !facts.open || facts.rate != Some(req.quote.rate) {
                    return Err(KioskError::RateChangedOrClosed);
                }

                // 6. Ok: status = Waiting, event = Request.
                let mut next_req = req.clone();
                next_req.status = KioskStatus::Waiting;
                Ok(Transition {
                    request: next_req,
                    event: Some(KioskEventKind::Request),
                })
            }

            KioskCommand::Accept => {
                // 1. authorize(Accept, actor) fails -> StaffRequired.
                Self::authorize(KioskAction::Accept, actor)?;
                let username = match actor {
                    KioskActor::Staff { username, .. } => username,
                    _ => return Err(KioskError::StaffRequired),
                };

                // 2. request is None, or its status is not Waiting -> AlreadyAssignedOrUnavailable.
                let req = match request {
                    Some(r) if r.status == KioskStatus::Waiting => r,
                    _ => return Err(KioskError::AlreadyAssignedOrUnavailable),
                };

                // 3. Ok: status = Accepted, staff = Some(me); event Accepted.
                let mut next_req = req.clone();
                next_req.status = KioskStatus::Accepted;
                next_req.staff = Some(username.clone());
                Ok(Transition {
                    request: next_req,
                    event: Some(KioskEventKind::Accepted),
                })
            }

            KioskCommand::IdentityChecked { confirmed } => {
                // 1. authorize(IdentityChecked, actor) fails -> StaffRequired.
                Self::authorize(KioskAction::IdentityChecked, actor)?;
                let username = match actor {
                    KioskActor::Staff { username, .. } => username,
                    _ => return Err(KioskError::StaffRequired),
                };

                // 2. confirmed is false -> PassportCheckUnconfirmed.
                if !confirmed {
                    return Err(KioskError::PassportCheckUnconfirmed);
                }

                let req = match request {
                    Some(r) => r,
                    None => return Err(KioskError::PassportCheckUnconfirmed),
                };

                // Check if I hold the request
                let holds = req.staff.as_deref() == Some(username.as_str());

                // 3. I hold the request and its status is Accepted -> Ok: status = IdChecked; event IdChecked.
                if holds && req.status == KioskStatus::Accepted {
                    let mut next_req = req.clone();
                    next_req.status = KioskStatus::IdChecked;
                    return Ok(Transition {
                        request: next_req,
                        event: Some(KioskEventKind::IdChecked),
                    });
                }

                // 4. I hold the request and its status is IdChecked -> unchanged (a repeat).
                if holds && req.status == KioskStatus::IdChecked {
                    return Ok(Transition {
                        request: req.clone(),
                        event: None,
                    });
                }

                // 5. anything else -> PassportCheckUnconfirmed.
                Err(KioskError::PassportCheckUnconfirmed)
            }
        }
    }

    pub fn identity(
        request: Option<&KioskRequest>,
        now: KioskTime,
    ) -> Result<RequestIdentity, KioskError> {
        let req = request.ok_or(KioskError::RequestNotFound)?;
        let first_eight: String = req.id.chars().take(8).collect();
        let queue = format!("K-{}", first_eight.to_uppercase());
        let order_no = format!("ORD-{}", req.id.to_uppercase());
        let transaction_no = req.transaction.as_ref().map(|t| t.document_ref.clone());

        let age = now.epoch_millis() - req.created.epoch_millis();
        let status = if req.status == KioskStatus::Quoted && age > 300_000 {
            KioskStatus::Expired
        } else {
            req.status
        };

        Ok(RequestIdentity {
            id: req.id.clone(),
            queue,
            order_no,
            transaction_no,
            status,
        })
    }

    pub fn check_get(request: Option<&KioskRequest>, actor: &KioskActor) -> Result<(), KioskError> {
        Self::authorize(KioskAction::Get, actor)?;
        let staff_name = match actor {
            KioskActor::Staff { username, .. } => username,
            _ => return Err(KioskError::StaffRequired),
        };
        if let Some(req) = request {
            if req.staff.as_deref() == Some(staff_name.as_str())
                && (req.status == KioskStatus::Accepted || req.status == KioskStatus::IdChecked)
            {
                return Ok(());
            }
        }
        Err(KioskError::AcceptFirst)
    }

    pub fn check_publish(
        request: Option<&KioskRequest>,
        actor: &KioskActor,
    ) -> Result<(), KioskError> {
        Self::authorize(KioskAction::PublishReceipt, actor)?;
        let staff_name = match actor {
            KioskActor::Staff { username, .. } => username,
            _ => return Err(KioskError::StaffRequired),
        };
        if let Some(req) = request {
            if req.staff.as_deref() == Some(staff_name.as_str())
                && (req.status == KioskStatus::IdChecked || req.status == KioskStatus::Completed)
            {
                return Ok(());
            }
        }
        Err(KioskError::AssignedStaffRequired)
    }

    pub fn check_save(
        request: Option<&KioskRequest>,
        save: &SaveAttempt,
        actor: &KioskActor,
    ) -> Result<(), KioskError> {
        let staff_name = match actor {
            KioskActor::Staff { username, .. } => username,
            KioskActor::Customer => return Err(KioskError::NotReadyToSave),
        };
        let req = match request {
            Some(r) => r,
            None => return Err(KioskError::NotReadyToSave),
        };
        if req.staff.as_deref() != Some(staff_name.as_str()) {
            return Err(KioskError::NotReadyToSave);
        }
        match req.status {
            KioskStatus::IdChecked => {}
            KioskStatus::Completed => match &req.transaction {
                Some(link) if link.submission_id == save.submission_id => {}
                _ => return Err(KioskError::NotReadyToSave),
            },
            _ => return Err(KioskError::NotReadyToSave),
        }

        if !save.live {
            return Err(KioskError::SaveMustBeLive);
        }

        if save.currency != req.quote.currency
            || save.side != req.quote.side
            || save.amount != req.quote.foreign
        {
            return Err(KioskError::SaveDiffersFromQueue);
        }

        if save.method != req.quote.payment_method.as_str() {
            return Err(KioskError::SaveMethodDiffers);
        }

        Ok(())
    }

    pub fn complete(
        request: &KioskRequest,
        submission_id: &str,
        document_ref: &str,
    ) -> Result<Transition, KioskError> {
        match request.status {
            KioskStatus::IdChecked => {
                let mut next_req = request.clone();
                next_req.status = KioskStatus::Completed;
                if next_req.transaction.is_none() {
                    next_req.transaction = Some(TransactionLink {
                        submission_id: submission_id.to_string(),
                        document_ref: document_ref.to_string(),
                    });
                }
                Ok(Transition {
                    request: next_req,
                    event: Some(KioskEventKind::Completed),
                })
            }
            KioskStatus::Completed => Ok(Transition {
                request: request.clone(),
                event: None,
            }),
            _ => Err(KioskError::NotReadyToSave),
        }
    }

    pub fn is_purgeable(request: &KioskRequest, now: KioskTime) -> bool {
        let age = now.epoch_millis() - request.created.epoch_millis();
        request.status == KioskStatus::Quoted && age > 600_000
    }

    pub fn receipt_readable(published: KioskTime, now: KioskTime) -> bool {
        now.epoch_millis() - published.epoch_millis() < 600_000
    }
}
