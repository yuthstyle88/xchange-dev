//! Booking members use cases (issue_member, members).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

use crate::domain::booking::commands::issue_member::IssueMember;
use crate::domain::booking::entities::member::Member;
use crate::domain::booking::repositories::member_repository::TokenTaken;
use crate::domain::booking::value_objects::booking_error::BookingError;
use crate::domain::booking::value_objects::booking_input::token_hash;
use xc_customer::application::customer::service::NO_SUCH_BRANCH;

use super::service::BookingService;

impl BookingService {
    pub async fn issue_member(
        &self,
        branch: &BranchId,
        cmd: IssueMember,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Member> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        if !cmd.identity_confirmed {
            return Err(AppError::Domain(
                BookingError::CustomerNotIdentified.to_string(),
            ));
        }

        let live = self.ports.live_customer(branch, &cmd.customer_id).await?;
        let customer = match live {
            Some(c) => c,
            None => {
                return Err(AppError::Domain(
                    BookingError::CustomerNotIdentified.to_string(),
                ));
            }
        };

        let b = branch.clone();
        let actor_str = actor.to_string();
        let members = self.members.clone();
        let ids = self.ids.clone();

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let member_id = ids.member_id();
                    let member = Member {
                        id: member_id,
                        branch_id: b.clone(),
                        customer_id: cmd.customer_id.clone(),
                        name: customer.name.clone(),
                        phone: cmd.phone.clone(),
                        token_hash: token_hash(&cmd.secret),
                        verified_at: now,
                        verified_by: actor_str,
                    };

                    let stored = members.upsert_in(tx, &member).await?;
                    match stored {
                        Ok(m) => Ok(m),
                        Err(TokenTaken) => {
                            Err(AppError::Domain(BookingError::BadMemberCode.to_string()))
                        }
                    }
                })
            })
            .await
    }

    pub async fn members(&self, branch: &BranchId) -> AppResult<Vec<Member>> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        self.members.list(branch).await
    }
}
