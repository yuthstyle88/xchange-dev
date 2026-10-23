# Design: every typed value is checked at the field, and every refusal names its field

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-23 ICT, from a read-only audit of every input in `web/src`
> (staff, admin, desk and customer screens; about 75 files with an `<input>`, `<textarea>` or `<select>`, plus the custom fields) at main `73b6727f`.
> Owner's order 2026-10-23: *"ต้อง fix การคีย์ การตรวจสอบการคย์ ให้ ครบ"* (input entry and its validation must be complete).
> The owner approved opening the tickets below on 2026-10-23. The questions in §5 are **not** ticketed until the owner rules on them.

Excluded because they are already handled: #4811 (book amount), #4826 (cooldown floor), #4842 (Simulate scenarios), #4844 (starting prices first load),
`web-silent-failures.md` §1-§9, and `blueprint/user-management-rbac.md` §11.10.

## 1. Root causes

| # | Root | Measured on `73b6727f` | Effect a person sees |
|---|---|---|---|
| RC-1 | **No field-aware wording on the web.** A v3 refusal is `400 {code:"validationFailed", field, message:"ข้อมูลไม่ถูกต้อง"}` (`api/src/presentation/http/v3/org_view.rs:94`). There is no shared `error.validationFailed` key, and `errorText` (`web/src/shared/i18n/index.ts:102-118`) falls back to the server's message. Only `useForm` (`web/src/shared/ui/form.ts:120-136`), StartingPrices, CashLimits and Book's referral field read `err.field`. | Two screens keep a local generic key (`booking-slots/i18n.ts:42`, `members/i18n.ts:33`). Identity, kiosk and verification codes have no web wording. | Thai `ข้อมูลไม่ถูกต้อง` in en and zh, raw English developer text (`cash.rs:60-67`, pricing, rate policy), or Thai-only `{error:"…"}` sentences, in a banner, never at the field. |
| RC-2 | **No web mirror of the kernel bounds.** The server has one core: `Bound` / `IntBound` (`api/crates/xc-kernel/src/money/bounded.rs`) under `Money`, `Rate`, `RatePercent` and `ForeignAmount` (`blueprint/bounded-inputs.md` B-1/B-2/B-7). Each screen hand-writes a regex or uses `Number` / `parseInt`. | Screens trim and then send untrimmed (`RatesDraftCard.tsx:28-31` vs `:269`), accept `+` (`RatesPolicyCard.tsx:95-100`), allow 8 decimals for a 2-decimal value (`campaign-rules.ts:41`), or coerce `'0x10'` to 16 (`KioskCurrenciesScreen.tsx:350-363`). | Values pass the screen and are refused later, or are silently changed. |
| RC-3 | **Hand-written server refusals that skip `v3::input`** (B-3): English text and `field: None` (`pricing_approve.rs`, `pricing_configure.rs`, `cash.rs`, `branch_save.rs:139-141`, `customer_phone_start.rs:85-93`). | The web cannot put them at a field even with RC-1 fixed. | Raw English with no field; "Server error" for a mistyped phone. |
| RC-4 | **Native validation bypassed.** `noValidate` forms, `type="button"` submits and missing `required` (`TierEditor.tsx:189`, `PricingPolicy.tsx:130`, `PermissionMatrix.tsx:453-458`, `Book.tsx:431,547`). | `required` and `maxLength` are written but never enforced. | Blank values are sent. |

## 2. T0: the field-error contract (#4897)

```ts
// web/src/shared/i18n/field-error.ts
export interface FieldMessage { field?: string; message: string }
export function fieldError(err: unknown, labels: Readonly<Record<string, string>>): FieldMessage
```
1. Not an `ApiError`, or `isServerFailure(err)` → `{ message: wordedError(err) }`.
2. A shared key `input.code.<code>` exists → that text; `field` = `err.field` when it is a key of `labels`.
3. `validationFailed` with `err.field` in `labels` → `t('input.invalid', { field: labels[field] })` at that field.
4. `validationFailed` otherwise → `t('input.invalidNoField')`, no field.
5. Anything else → `wordedError(err)`, with `field` when it is a key of `labels`.

Keys (th, en, zh): `input.invalid` (`{field}`), `input.invalidNoField`, `input.code.invalidPhone`, `input.code.phoneRequired`,
`input.code.invalidCode`, `input.code.codeExpired`, `input.code.tooManyAttempts`, `input.code.invalidCredentials`,
`input.code.nameTooLong`, `input.code.quoteRefused`, `input.code.expiresAtNotInFuture`, and `kioskCancel.noteRequired` /
`kioskCancel.noteTooLong` (`{max}`) for T2.

**`wordedError`, `errorText` and `form.ts` do not change**, so no existing `worded.test.tsx` row moves. Each screen ticket
switches its own refusal path to `fieldError` and passes its own labels (the visible field names from its i18n).

## 3. T1: the bounded mirror (#4898)

`web/src/shared/ui/bounded.ts` is pure (no DOM, no Solid). It has `numeric(precision, scale, sign)`, `atMost`, `checkDecimal`, `checkInteger`, and presets
`MONEY` (non-negative, 2 dp, ≤ 100000000), `RATE` (positive, 8 dp, ≤ 100000000), `RATE_PERCENT` (numeric(9,4) signed, one
leading `+` allowed, as `rate_percent.rs`) and `foreignAmount(minorUnit)` (positive, `minorUnit` dp, ≤ 100000000).
Rules, exactly as `Bound::parse`:
- **No trim.** A caller that trims sends the trimmed text it checked.
- Digits and at most one `.`, never leading or trailing. No exponent, comma or space.
- `-` only when the sign is signed; `+` only for `RATE_PERCENT`.
- Extra decimals are accepted only when they are zeros.
- Magnitude is compared on the digit string, never through `Number`.

`ok.text` is what the screen sends. A server-side ts-rs export of the presets is a possible later step; until then the
presets are pinned by tests copied from the kernel's pinned cases.

## 4. T2: the reason-dialog rule (#4903)

A reason dialog states its rule before the person confirms:
- `required` refuses blank (after trim) with a worded line under the textarea and does not call `onConfirm`;
- `maxLength` is on the textarea, with an `n / max` counter;
- the dialog stays open until the request is sent.

The bounds come from the server: reversal 1..500 (`reversal_request.rs:58-64`), hold reason and release note 500 (`branch_day/service.rs:405,459`, which otherwise cut silently), kiosk cancel 200 with a note for OTHER (`cancel_reason.rs:26-50`).
A dialog with no props behaves as today.

## 5. Owner questions (not ticketed)

These have no rule on the server, or the rule is undefined. Nothing is built for them until the owner decides:
- **`/app/book` side:** the label "ซื้อเงินต่างประเทศ" ("buy foreign currency") sends `shopSide: BUY`, which `bookingCreate` reads as the shop buying (`create.rs:146-148`). Does the label mean the customer's side? This is asked inside #4904.
- **Booking quantity:** is a booking under 1 unit allowed (`Book.tsx` `min="1"` vs the server's `Positive`)? Is 100,000,000 the cap for bookings as for the preview (`create.rs:164` vs `planner.rs:24`)? Should the desk booking quantity use `ForeignAmount` (minor units)?
- **Desk reference rate:** it is typed and never sent (`desk-store.ts:181`). Send it or remove it.
- **Floor targets:** should the floor targets be checked per currency (`ForeignAmount`) instead of `Money` (`branch_day.rs:243-259`)?
- **Transfers:** may a transfer have zero lines (`transfer_flow.rs`; counts already refuse it, `count_service.rs:888`)?
- **Text lengths with no bound** (the server takes any length): transfer and reconcile notes (the screen silently cuts at 200), reconciliation certify note, approval reject reason (the server does not even require it, `approval_decide.rs:28-98`), user name and profile fields, branch code and address, consent body, segment name, payment ref, account and notes, booking display name, ticket and booking cancel reason, assistant message, closure reason.
- **Number ranges with no bound:** rate-alert-settings cooldown, max active and lifetime (i32 only), booking-slot capacity and policy minutes (i64 max on i32), slot capacity 0 (the server takes it, the screen refuses it), campaign priority, coupon count per issue (u32, one row each), segment `value_thb` (a domain `Decimal` that the bounded-inputs guard does not scan), rate-policy guards and maximum reference age, consent retention maximum.
- **Meaning of an input:**
  - an empty consent retention field clears the policy (`Consent.tsx:41-42`), and no screen reads the current value;
  - the campaign detail name is editable but never saved (`Campaigns.tsx:513-522`);
  - a blank stack group becomes `DEFAULT`;
  - the rate-alert expiry: a day or a time;
  - the stale limit's sign (the server accepts ≤ 0);
  - a closure date in the past;
  - the coupon expiry against the campaign end;
  - the username format and the Identity register error contract (`http_identity_client.rs:176` maps unknown codes to 503);
  - the OTP code format;
  - one shared phone rule (booking: normalize only; phone verify: 8-15 digits).
- **Loyalty rule fields:** the bounds belong to the Loyalty service contract.

## 6. Tickets

Opened 2026-10-23, owner-approved. File scopes are disjoint (checked: no path in two scopes). Logic tickets get up-front tests on a
`feat/` base before release. "Needs merged first" is the order; it is written in each ticket's §1 in plain words.

| Ticket | Audit id | Side | Class | File scope | Needs merged first |
|---|---|---|---|---|---|
| #4897 | T0 | FE | logic | `web/src/shared/i18n/field-error.ts`<br>`web/src/shared/i18n/th.ts`<br>`web/src/shared/i18n/en.ts`<br>`web/src/shared/i18n/zh.ts`<br>`web/src/shared/i18n/field-error.test.ts` | — |
| #4898 | T1 | FE | logic | `web/src/shared/ui/bounded.ts`<br>`web/src/shared/ui/bounded.test.ts` | — |
| #4903 | T2 | FE | mechanical | `web/src/screens/staff/parts/ReasonDialog.tsx`<br>`web/src/screens/staff/parts/ReasonDialog.bounds.test.tsx`<br>`web/src/screens/staff/i18n.ts`<br>`web/src/shared/ui/CancelReasonDialog.tsx`<br>`web/src/screens/desk-transaction/TransactionScreen.tsx`<br>`web/src/screens/staff/holds/HoldsCard.tsx`<br>`web/src/screens/staff/holds/i18n.ts` | #4897 |
| #4904 | T3 | FE | logic | `web/src/screens/customer/book/Book.tsx`<br>`web/src/screens/customer/book/i18n.ts`<br>`web/src/screens/customer/book/Book.input-validation.test.tsx`<br>`web/src/screens/customer/book/Book.client.test.tsx`<br>`web/src/screens/customer/book/Book.keyed.test.tsx`<br>`web/src/screens/customer/book/Book.submit-validity.test.tsx`<br>`web/src/screens/customer/book/Book.preview.test.tsx` | #4897, #4898, #4899; owner answer on the side label |
| #4899 | T3b | BE | logic | `api/src/presentation/http/v3/views/booking_slots.rs`<br>`api/src/presentation/http/v3/commands/bookings/create.rs`<br>`api/src/presentation/http/v3/commands/bookings/reschedule.rs`<br>`web/src/shared/api/generated/bookings.ts`<br>`api/tests/booking_past_slot_test.rs` | — |
| #4905 | T4 | FE | logic | `web/src/screens/desk/steps/rate/step.tsx`<br>`web/src/screens/desk/steps/rate/i18n.ts`<br>`web/src/screens/desk/desk-store.ts`<br>`web/src/screens/desk/steps/payment/step.tsx`<br>`web/src/screens/desk/steps/payment/i18n.ts`<br>`web/src/screens/desk/steps/booking/step.tsx`<br>`web/src/screens/desk/steps/booking/i18n.ts`<br>`web/src/screens/desk/steps/input-validation.test.tsx`<br>`web/src/screens/desk/steps/rate/step.test.tsx`<br>`web/src/screens/desk/worded-errors.test.tsx`<br>`web/src/screens/desk/desk-store.cleared.test.ts` | #4897, #4898 |
| #4906 | T5 | BE | logic | `api/src/presentation/http/v3/commands/branch_day.rs`<br>`api/tests/draft_begin_foreign_amount_test.rs` | — |
| #4900 | T6 | BE | mechanical | `api/src/presentation/http/v3/commands/customer_phone_start.rs`<br>`api/src/presentation/http/v3/commands/customer_phone_confirm.rs`<br>`api/tests/customer_phone_refusal_message_test.rs` | — |
| #4907 | T7 | FE | mechanical | `web/src/screens/desk/loyalty/phone/PhoneVerify.tsx`<br>`web/src/screens/desk/loyalty/phone/i18n.ts`<br>`web/src/screens/desk/loyalty/phone/PhoneVerify.input.test.tsx`<br>`web/src/screens/desk/referral/ReferralCodeField.tsx`<br>`web/src/screens/desk/referral/i18n.ts`<br>`web/src/screens/desk/referral/ReferralCodeField.input.test.tsx` | — |
| #4908 | T8 | FE | mechanical | `web/src/screens/desk/steps/customer/step.tsx`<br>`web/src/screens/desk/steps/customer/i18n.ts`<br>`web/src/screens/desk/steps/customer/step.bounds.test.tsx` | — |
| #4909 | T9 | FE | logic | `web/src/screens/desk/bookings/DeskBookingsScreen.tsx`<br>`web/src/screens/desk/bookings/i18n.ts`<br>`web/src/screens/desk/bookings/DeskBookings.input-validation.test.tsx`<br>`web/src/screens/desk/bookings/DeskBookings.new-booking-dialog.test.tsx` | #4897 |
| #4910 | T10 | FE | logic | `web/src/screens/staff/shift/ShiftCard.tsx`<br>`web/src/screens/staff/shift/i18n.ts`<br>`web/src/screens/staff/transfers/TransfersCard.tsx`<br>`web/src/screens/staff/transfers/i18n.ts`<br>`web/src/screens/admin/cash/CashFlows.tsx`<br>`web/src/screens/admin/cash/i18n.ts`<br>`web/src/screens/staff/cash-quantities.input-validation.test.tsx`<br>`web/src/screens/staff/shift/ShiftCard.nolines.test.tsx`<br>`web/src/screens/admin/cash/CashFlows.keyed.test.tsx` | #4897, #4898, #4901 |
| #4901 | T10b | BE | logic | `api/src/presentation/http/v3/commands/cash.rs`<br>`api/src/presentation/http/v3/commands/cash/transfers.rs`<br>`api/src/presentation/http/v3/commands/cash/handovers.rs`<br>`api/crates/xc-cash/src/application/transfer/transfer_flow.rs`<br>`api/tests/cash_input_fields_test.rs` | — |
| #4902 | T11 | BE | logic | `api/src/presentation/http/v3/commands/pricing_approve.rs`<br>`api/src/presentation/http/v3/commands/pricing_configure.rs`<br>`api/tests/pricing_input_fields_test.rs`<br>`api/tests/pricing_approve_test.rs`<br>`api/tests/pricing_approve_tier_check_test.rs`<br>`api/tests/pricing_configure_test.rs` | — |
| #4911 | T12 | FE | logic | `web/src/screens/admin/pricing/TierEditor.tsx`<br>`web/src/screens/admin/pricing/BulkApprove.tsx`<br>`web/src/screens/admin/pricing/PricingPolicy.tsx`<br>`web/src/screens/admin/pricing/i18n.ts`<br>`web/src/screens/admin/pricing/Pricing.input-validation.test.tsx`<br>`web/src/screens/admin/pricing/PricingPolicy.test.tsx`<br>`web/src/screens/admin/pricing/TierEditor.test.tsx`<br>`web/src/screens/admin/pricing/BulkApprove.test.tsx` | #4897, #4898, #4902 |
| #4912 | T13 | FE | logic | `web/src/screens/admin/rates/RatesDraftCard.tsx`<br>`web/src/screens/admin/rates/RatesPolicyCard.tsx`<br>`web/src/screens/admin/rates/i18n.ts`<br>`web/src/screens/admin/rates/Rates.input-validation.test.tsx`<br>`web/src/screens/admin/rates/RatesTiers.test.tsx`<br>`web/src/screens/admin/rates/RatesManualPrice.test.tsx` | #4897, #4898 |
| #4913 | T14 | FE | mechanical | `web/src/screens/admin/starting-prices/StartingPrices.tsx`<br>`web/src/screens/admin/starting-prices/i18n.ts`<br>`web/src/screens/admin/starting-prices/StartingPrices.bounds.test.tsx` | — |
| #4914 | T15 | FE | mechanical | `web/src/screens/admin/alert-settings/AlertSettings.tsx`<br>`web/src/screens/admin/alert-settings/i18n.ts`<br>`web/src/screens/admin/alert-settings/AlertSettings.bounds.test.tsx` | — |
| #4915 | T16 | FE | logic | `web/src/screens/admin/campaigns/Campaigns.tsx`<br>`web/src/screens/admin/campaigns/campaign-rules.ts`<br>`web/src/screens/admin/campaigns/CampaignRulesEditor.tsx`<br>`web/src/screens/admin/campaigns/i18n.ts`<br>`web/src/screens/admin/campaigns/Campaigns.input-validation.test.tsx`<br>`web/src/screens/admin/campaigns/Campaigns.test.tsx`<br>`web/src/screens/admin/campaigns/Campaigns.numbers.test.tsx`<br>`web/src/screens/admin/campaigns/campaign-rules.test.ts` | #4897, #4898 |
| #4916 | T16b | BE | logic | `api/src/presentation/http/v3/commands/campaign_version_save.rs`<br>`api/tests/campaign_rule_eligibility_bound_test.rs` | — |
| #4917 | T17 | FE | mechanical | `web/src/screens/admin/loyalty/Loyalty.tsx`<br>`web/src/screens/admin/loyalty/i18n.ts`<br>`web/src/screens/admin/loyalty/Loyalty.expiry-months.test.tsx` | — |
| #4918 | T18 | FE | mechanical | `web/src/screens/admin/transactions/Transactions.tsx`<br>`web/src/screens/admin/reports/Reports.tsx`<br>`web/src/screens/admin/executive/Executive.tsx`<br>`web/src/screens/admin/executive/i18n.ts`<br>`web/src/screens/admin/transactions/Transactions.filters.test.tsx`<br>`web/src/screens/admin/executive/Executive.currency-select.test.tsx`<br>`web/src/screens/admin/executive/Executive.test.tsx` | — |
| #4919 | T19 | FE | mechanical | `web/src/screens/admin/reconciliation/Reconciliation.tsx`<br>`web/src/screens/admin/reconciliation/i18n.ts`<br>`web/src/screens/admin/payments/Payments.tsx`<br>`web/src/screens/admin/payments/i18n.ts`<br>`web/src/screens/devices/DevicesScreen.tsx`<br>`web/src/screens/devices/i18n.ts`<br>`web/src/screens/kiosk-currencies/KioskCurrenciesScreen.tsx`<br>`web/src/screens/kiosk-currencies/i18n.ts`<br>`web/src/screens/admin/booking-slots/BookingSlots.tsx`<br>`web/src/screens/admin/admin-inputs.bounds.test.tsx`<br>`web/src/screens/kiosk-currencies/KioskCurrencies.result-seconds.test.tsx`<br>`web/src/screens/admin/payments/Payments.test.tsx`<br>`web/src/screens/admin/booking-slots/BookingSlots.worded.test.tsx` | — |
| #4920 | T20 | FE | logic | `web/src/screens/admin/users/UsersScreen.tsx`<br>`web/src/screens/admin/users/PermissionMatrix.tsx`<br>`web/src/screens/admin/users/i18n.ts`<br>`web/src/shared/ui/PasswordField.tsx`<br>`web/src/screens/login/ChangePassword.tsx`<br>`web/src/screens/login/i18n.ts`<br>`web/src/screens/admin/users/Users.input-validation.test.tsx`<br>`web/src/screens/admin/users/UsersScreen.a6.test.tsx`<br>`web/src/screens/admin/users/PermissionMatrix.test.tsx` | #4897 |
| #4921 | T21 | FE + BE | logic | `api/src/presentation/http/v3/commands/branch_save.rs`<br>`api/src/presentation/http/branch_tablets.rs`<br>`api/src/infrastructure/paired_devices/pairing.rs`<br>`api/tests/branch_save_field_test.rs`<br>`web/src/screens/admin/branches/BranchesScreen.tsx`<br>`web/src/screens/admin/branches/BranchTablets.tsx`<br>`web/src/screens/admin/branches/i18n.ts`<br>`web/src/screens/admin/branches/Branches.input-validation.test.tsx`<br>`web/src/screens/admin/branches/BranchesScreen.test.tsx` | #4897 (screen half) |
| #4922 | T22 | FE | logic | `web/src/screens/admin/members/Members.tsx`<br>`web/src/screens/admin/members/i18n.ts`<br>`web/src/screens/admin/consent/Consent.tsx`<br>`web/src/screens/admin/consent/i18n.ts`<br>`web/src/screens/admin/members/Members.input-validation.test.tsx`<br>`web/src/screens/admin/members/Members.rule-int.test.tsx` | #4897 |
| #4923 | T23 | FE | logic | `web/src/screens/customer/rate-alerts/RateAlerts.tsx`<br>`web/src/screens/customer/rate-alerts/i18n.ts`<br>`web/src/screens/customer/queue/RequestForm.tsx`<br>`web/src/screens/customer/queue/i18n.ts`<br>`web/src/screens/customer/rate-alerts/RateAlerts.input-validation.test.tsx`<br>`web/src/screens/customer/queue/RequestForm.input-validation.test.tsx`<br>`web/src/screens/customer/rate-alerts/RateAlerts.keyed.test.tsx` | #4897, #4898 |
| #4924 | T24 | FE | mechanical | `web/src/screens/customer/sign-in/SignIn.tsx`<br>`web/src/screens/customer/sign-in/SignUp.tsx`<br>`web/src/screens/customer/sign-in/i18n.ts`<br>`web/src/screens/customer/sign-in/SignInUp.input.test.tsx`<br>`web/src/screens/customer/sign-in/worded.test.tsx` | #4897 |
| #4925 | T25 | FE + BE | logic | `api/crates/xc-verification/src/application/case_service.rs`<br>`api/src/presentation/http/branch_verification_intake.rs`<br>`api/tests/verification_empty_file_test.rs`<br>`web/src/screens/customer/verification/VerificationScreen.tsx`<br>`web/src/screens/customer/verification/i18n.ts`<br>`web/src/screens/operations/verification/VerificationScreen.tsx`<br>`web/src/screens/operations/verification/i18n.ts`<br>`web/src/screens/customer/verification/Verification.file-check.test.tsx`<br>`web/src/screens/customer/verification/VerificationUploadErrors.test.tsx` | #4897 (screen half) |

## 7. Data migration
None. No table, column, constraint or stored value changes.
