# Design: customer screens must not fail silently (round 2)

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-22 13:25 ICT, from a read-only audit of `web/src` at main
> `fa4163ab` (743 non-test files; ~25 candidates read in depth). Round 1 was `web-public-get.md` (the host read).
> Owner rule: bugs only. Every row below is a defect a person can hit, with its root cause.

## 1. Bugs and root causes

| # | Screen | What the person sees | Root cause |
|---|---|---|---|
| B1 | `/app/queue` status (phone and tablet) | **First status read fails:** `Status:` with nothing after it. **A later read fails:** the last status and number stay frozen, with no sign that they are stale. It retries silently forever, and a phone `401` is never shown. | `shared/live/view-poller.ts:111-115`: the poller's catch only backs off, and it has no error or last-OK accessor (`PublicPoller` has `lastOkAt`). `RequestStatus.tsx:39-51` acts only on a `404` after `expiresAt`. |
| B2 | `/app/queue?request=<id>`, reloaded | The reissue fails (5xx, network or 401), and the page shows an empty form, as if the request were gone. A second submit is then refused with `409 alreadyQueued`, and a `401` shows the form instead of the sign-in path. | `customer/queue/queue-state.ts:119-124`: `catch { showForm(false) }` treats every failure as the `404` "request gone" case. |
| B4 | `/app/book`, slots | A slots answer that is not JSON (e.g. an HTML fallback page) shows "no slots", with no error and no retry. | `customer/book/Book.tsx:222-233`: a raw `fetchFn` read of `/api/v3/views/booking_slots` with `.json().catch(() => ({}))`. `api.view('booking_slots', …)` builds the same URL and throws on a parse failure. |

## 2. Decisions

| # | Decision |
|---|---|
| SF-1 | **`ViewPoller` reports its own failures** (♻️, the root of B1): it gains `error(): ApiError \| null` (the last failure, cleared by the next good read) and `lastOkAt(): number \| null`, matching `PublicPoller`. A `401` is kept as `error()` with status 401. Backoff and polling behaviour do not change. No consumer needs its own wrapper to see a failure. |
| SF-2 | **`RequestStatus` shows the failure** (B1): if no read has succeeded yet and `error()` is set, it shows a worded "status unavailable, retrying" line, never a blank label. If a read succeeded earlier and `error()` is set now, it keeps the last status and adds a worded "last updated hh:mm, reconnecting" line (`lastOkAt`). A `401` shows the sign-in link (the U1 path). It stays silent after `expiresAt`, as today. |
| SF-3 | **Reissue on reload tells "gone" from "failed"** (B2): only `404` means the request is gone, and shows the form as today. A `401` means signed out, through the existing U1 path. Anything else shows a worded error with a retry that reissues again, keeping the request id. It never shows the empty form. |
| SF-4 | **Book reads slots through `api.view`** (B4): `api.view('booking_slots', {branch, filter:{date}})`. A failure, or a body that is not JSON, shows the screen's existing `error.loadFailed` plus retry, never "no slots". |

## 3. Tickets (File scopes disjoint)

| Ticket | Scope | Class | Needs |
|---|---|---|---|
| SF-a (B1 core) | `web/src/shared/live/view-poller.ts`, `web/src/shared/live/view-poller.error.test.ts` | logic | — |
| SF-b (B1 + B2 screen) | `web/src/screens/customer/queue/RequestStatus.tsx`, `web/src/screens/customer/queue/queue-state.ts`, `web/src/screens/customer/queue/i18n.ts`, `web/src/screens/customer/queue/RequestStatus.offline.test.tsx`, `web/src/screens/customer/queue/queue-state.reissue-error.test.ts` | logic | SF-a, for the B1 half |
| SF-c (B4) | `web/src/screens/customer/book/Book.tsx`, `web/src/screens/customer/book/Book.slots-parse.test.tsx` | mechanical | — |

## 4. Not in this round
- **B3, the sign-up terms text:** moved to §6 (owner decision 2026-10-22: A).
- **B5, pricing `me` failure** (`Pricing.tsx:80-87`: `api.view('me')…catch(() => {})`, the #1719 pattern). Not fixed in this round. The admin pricing screen is under the pricing work of #4324 (its body names `web/src/screens/admin/pricing/TierEditor.tsx` in the same folder). No File scope at fa4163ab claims `Pricing.tsx` yet.
- **G-ME:** three staff screens pin a silent hide when `me` fails. Designed in §5.
- **Already decided as "not a bug"** (so they are not re-audited): `BranchLive.tsx:33,45`, `save.ts:213,275`, `ReceiptFrame.tsx:47`, `tablet.ts:48`, `Shell.tsx:65`, `DeskBookingsScreen.tsx:212`, and the `UsersScreen` and `VerificationScreen` fallbacks pinned by their designs.

## 5. G-ME: a failed `me` read is worded, never a silent hide

> designed-by: claude/e8020026 (XC FE lane controller), through a sub-agent, 2026-10-22 ICT, re-measured on main
> `210987c1`. Skeleton and up-front tests: branch `feat/gme` (from `210987c1`).

### 5.1 Bug and root cause

| Screen | What the person sees when `me` fails | Where the hide is made | Pinned by (retired in the skeleton) |
|---|---|---|---|
| B04 `/desk/transactions/{id}` | The receipts and the transaction show, but **no reprint and no reversal-request button**, with no word. A teller thinks they lack the permission. | `TransactionScreen.tsx:85-90` `.catch(() => undefined)`, then `can(undefined, …)` is false (`:169`, `ReceiptsSection.tsx:34`, `TransactionScreen.tsx:213`) | `TransactionScreen.test.tsx` "me failing still shows the data, with no action button" |
| Desk receipt step, referral field | The referral field is gone (`innerHTML === ''`). | `ReferralCodeField.tsx:46-58` (`setMe(undefined)`), `:87` | `ReferralCodeField.permission.test.tsx` "when me fails, the field is hidden" |
| `/desk` pending refunds (OWNER_DESK) | The "refunds to record" nav item is gone. | `steps/refunds/task.tsx:36-46` `.catch(() => {})` | `refunds.test.tsx` RF-9 "when me fails: no nav item" |

Root cause (♻️): each screen reads `me` itself and turns any failure into "no permission". The page's session store
(`shared/session/session.ts`, a `ViewPoller` on `me`) already reads `me` on every staff page, and its wrapper sees each
failure (`session.ts` `wrappedApi.view` catch) but exposes none. #1719 fixed one screen (`CashFlows.tsx:88-98`;
`Rates.tsx:169-183` is the same) with its own copy; there is no shared "me failed" state and no shared worded line.

Re-checked and **not** this bug: `VerificationScreen.tsx:270-290` (a failed `me` falls through to the queue read, which
the server gates; fail-closed on the role codes, pinned by its design) and `DeskBookingsScreen.tsx:192-213` (`me` only
prefills the branch; no action is gated on it).

### 5.2 Decisions

| # | Decision |
|---|---|
| GM-1 | **The session store knows when `me` failed.** `SessionStore.meError?(): ApiError \| null` and `retryMe?(): void`; `EndingSessionStore` (the real store) has both. `meError()` is the last failure of the store's own `me` read: a thrown `ApiError` as is, anything else as `ApiError{code:'network', status:0}` (SF-1's mapping), cleared by the next good read (`changed` true or false). `retryMe()` re-reads `me` at once **through the same poller** (no second request path; the next scheduled read is one interval after it); it does nothing after `signOut()` or once the session has ended. Nothing else about polling, `ended` or `me()` changes. Optional on `SessionStore` only so the many test fakes stay valid. |
| GM-2 | **One shared worded line.** `shared/ui/PermissionsUnavailable.tsx`: `PermissionsUnavailable({ error, onRetry, busy? })` renders nothing for a null/undefined error, else `<p role="alert" class="xc-perm-unavailable" data-permissions-unavailable>` with `permissionsUnavailableText(error)` and a `button[data-action="permissions-retry"]` (`t('shell.permissions.retry')`, disabled while `busy`). `permissionsUnavailableText(error)` = `t('shell.permissions.unavailable')` + ' ' + `wordedError(error)`; it is also the `title` of every disabled action. Keys `shell.permissions.unavailable` / `.retry` (th, en, zh) and `permissions-unavailable.css` are in the skeleton. |
| GM-3 | **"Permissions unknown" ≠ "no permission".** While the permissions are unknown (the `me` read failed and no good answer is held), a screen shows the shared line, and each permission-gated action is **on screen, disabled, with the reason as `title`**. Non-permission conditions still decide whether an action exists (e.g. a request button needs COMPLETED and no active reversal). Nothing gated is sent or read (e.g. `desk_refunds`). Once `me` loads, behaviour is exactly today's (table 5.3). A failed poll **after** a good `me` keeps today's behaviour: `me()` holds the last answer, so the permissions are known and no line shows. |
| GM-4 | **Who reads `me`.** The refunds task moves to the session store (`sharedSession().me()/.meError()/.retryMe()`): its tests boot the real page, which already reads `me` through the session. B04 and the referral field **keep their own `me` read**, because other kept rows pin it: B04's D2 (`desk-transaction-detail.md`) reads `me` on open and on refresh, with no timer (`TransactionScreen.test.tsx` rows "opens with exactly me, transaction and transaction_audit" (3 calls), "owner … gets('me') = 1", "makes no request in 10 minutes", "refresh reads the same three views once more"); the referral field reads it through its `api` prop (`ReferralCodeField.permission.test.tsx` rows 1-3 `toHaveBeenCalledWith('me')`, `ReferralCodeField.test.tsx` stub store, `ReferralCodeField.keyed.test.tsx` fetch stub). Those two keep the failure in their own state, and use GM-2's line and text; their retry re-reads `me` the way they already read it. Moving them onto the store is in 5.5. |

### 5.3 Behaviour per screen

| Screen | `me` loaded, permission missing (kept) | `me` failed (new) | Retry |
|---|---|---|---|
| B04 | reprint **hidden** (`ReceiptsSection.tsx:34`); request-reversal **hidden** (`TransactionScreen.tsx:213` via `:169`) | data shows (`ready`); line at the top of the ready view; one reprint per receipt + request-reversal (if COMPLETED, no pending/active reversal) **disabled**, `title` = reason; no GET/POST/print on click | `permissions-retry` reads `me` once more (the refresh button also re-reads it, D2) |
| Referral field | whole field **hidden** (`ReferralCodeField.tsx:87`, `innerHTML ''`) | `.desk-referral-field` shows with the line inside; input and `.desk-referral-btn` **disabled**, button `title` = reason; nothing sent | `permissions-retry` calls `api.view('me')` once more |
| Refunds task (OWNER_DESK) | nav item **hidden** (`task.tsx:50`), `desk_refunds` never read | nav item `button[data-desk-task="refunds"]` **disabled**, `title` = reason, outside `ol.steps`; line in `nav.desk-nav`; `desk_refunds` never read; step cannot open | `permissions-retry` → `retryMe()`; on a good answer the task reads `desk_refunds` once, as today. STAFF_DESK unchanged (O-8) |

### 5.4 Tickets (File scopes disjoint)

| Ticket | File scope | Class | Needs |
|---|---|---|---|
| GME-0 (store + shared line) | `web/src/shared/session/session.ts`<br>`web/src/shared/ui/PermissionsUnavailable.tsx` | logic | — |
| GME-T (B04) | `web/src/screens/desk-transaction/TransactionScreen.tsx`<br>`web/src/screens/desk-transaction/ReceiptsSection.tsx` | logic | GME-0 |
| GME-F (referral field) | `web/src/screens/desk/referral/ReferralCodeField.tsx` | logic | GME-0 |
| GME-R (refunds task) | `web/src/screens/desk/steps/refunds/task.tsx` | logic | GME-0 |

Up-front tests (on `feat/gme`, do not edit): GME-0 `shared/session/session.meError.test.ts` (14: 10 red, 4 guards) and
`shared/ui/PermissionsUnavailable.test.tsx` (9: 8 red, 1 guard); GME-T `desk-transaction/TransactionScreen.me-error.test.tsx`
(15: 10 red, 5 guards); GME-F `desk/referral/ReferralCodeField.me-error.test.tsx` (10: 8 red, 2 guards); GME-R
`desk/steps/refunds/refunds.me-error.test.tsx` (9: 5 red, 4 guards). The screen tickets' tests import GME-0's component
for real, so they turn green only on a base that holds GME-0.

Notes for the implementers:
- GME-0: record the failure in the existing `wrappedApi.view` catch for `name === 'me'` and clear it on its success
  path; `retryMe` may restart the poller (`stop()` then `start()` keeps its `since` version). When `feat/silent-failures`
  (SF-1, `ViewPoller.error()`) is on the base, `meError` may read `poller.error()` instead; the tests pin the behaviour,
  not the source.
- GME-T: `ReceiptsSection` gains an optional reason prop for the `title`; keep `can()` from `staff/parts/can.ts` (D8).
- GME-R: derive visibility with a boolean memo and read `desk_refunds` once when it first becomes true: the session
  answers `me` again every 60 s, and RF-10 pins one read. `steps/receipt/page-flow.test.tsx` feeds a `me` without
  `permissions`; the task must not throw on it (`can()` does), so read `me()?.permissions ?? []`.

### 5.5 Not in this ticket set
- **B5, `Pricing.tsx:80-87`** (`.catch(() => {})`): the same bug. #4324 (another controller) owns `Pricing.tsx`; it should
  take GM-1/GM-2 there after GME-0 lands.
- **B04 and the referral field still read `me` themselves** (GM-4). Moving them onto the session store needs the rows named
  in GM-4 retired or rewritten (B04's D2 changes from "the screen reads me" to "the session reads me"). That is a
  controller decision; the shared line and text do not change either way.

## 6. SF-5: sign-up waits for the TERMS text (owner 2026-10-22: A)

> designed-by: claude/e8020026 (XC FE lane controller), through a sub-agent, 2026-10-22 ICT, measured on main
> `ab456e93`. Owner decision 2026-10-22 ~15:25 ICT, verbatim "A": **sign-up is blocked until the TERMS text has loaded
> and is shown.** The alternative, warning only, was rejected. Skeleton and up-front tests: branch `feat/sf5-terms`.

### 6.1 Bug and root cause

| Screen | What the person sees | Root cause |
|---|---|---|
| B3 `/app/sign-up` | Any failure to read the TERMS text (host read, texts read, network, a body that is not an array) or no TERMS text in the page language shows the generic line "By signing up you accept the terms of service", and the submit button works. Sign-up records TERMS consent (the server writes the TERMS text of `lang`) to a text the person never saw. | `TermsText.tsx:33,36,37-38`: `loadTerms` turns every outcome other than "found" into `null`, and `null` shows the static `terms` line. `SignUp.tsx:88-89`: `<TermsText />` reports nothing to its parent, and the button is gated only on `pending()`. `TermsText` also reads through the global `fetch`, not the screen's. |

The server already refuses a sign-up with no TERMS text in `lang` (`ConsentTextMissing`,
`api/src/application/customer_binder.rs:25`), so `missing` is refused either way. The gap is `failed`: the text exists,
the person did not see it, and the server accepts.

### 6.2 Rule

| # | Decision |
|---|---|
| SF-5.1 | **`TermsText` reports a state** `TermsState = 'loading' \| 'ready' \| 'missing' \| 'failed'` through `onState?: (state: TermsState) => void`. It calls `onState('loading')` each time a read starts (on mount and on retry), then `onState(<outcome>)` when that read ends. Its existing props (`fetch`, `api`) keep working, and `onState` is optional. |
| SF-5.2 | **`ready`**: the TERMS item in the page language was found. Body and version are shown as today (`terms-body`, `terms-version`). |
| SF-5.3 | **`missing`**: a 200 answer that is an array with no TERMS item (`[]`, or only MARKETING). It shows `<p class="xc-sign-in-terms-missing" role="alert" data-testid="terms-missing">` with `consentTextMissing`, the sentence the server's refusal already shows for the same fact. No retry: changing the language is the remedy. |
| SF-5.4 | **`failed`**: the host read failed (a refusal, a network failure, or an answer with no org: `readHostOrg` throws), the texts read failed (non-2xx or network), or the texts answer is not an array. It shows `<div class="xc-sign-in-terms-failed-box">` holding `<p class="xc-sign-in-terms-failed" role="alert" data-testid="terms-failed">` with `termsFailed`, and `<button type="button" class="xc-sign-in-link" data-testid="terms-retry">` with `termsRetry`. The button is `type="button"`, so it never submits the form. A click loads again (host and texts), through the same path. |
| SF-5.5 | **`loading`** shows `<p class="xc-sign-in-terms" data-testid="terms-loading">` with `termsLoading`. **No state but `ready` shows the static `terms` line.** The `terms` key stays in `i18n.ts`, because the tests use it to prove it is not shown. |
| SF-5.6 | **`SignUp.tsx`** passes its `fetch` to `TermsText` (in the skeleton) and keeps the reported state in a signal that starts at `loading`. The submit button is `disabled={pending() \|\| state !== 'ready'}`, and `submit` returns before posting when the state is not `ready` (a form submit event, e.g. Enter in a field, must not post either). It never posts `/api/app/register` without a shown TERMS text. |

### 6.3 Ticket

| Ticket | File scope | Class | Needs |
|---|---|---|---|
| SF-5 | `web/src/screens/customer/sign-in/TermsText.tsx`<br>`web/src/screens/customer/sign-in/SignUp.tsx` | logic | — |

Skeleton (on `feat/sf5-terms`, do not edit): the `TermsState` type and the `onState` prop (not called yet);
`<TermsText fetch={props.fetch} />` in `SignUp.tsx`; keys `termsLoading`, `termsFailed`, `termsRetry` (th, en, zh) in
`customer/sign-in/i18n.ts`; classes `.xc-sign-in-terms-missing`, `.xc-sign-in-terms-failed`,
`.xc-sign-in-terms-failed-box` in `sign-in.css`. Retired, because the owner's decision supersedes them:
`TermsText.test.tsx` "falls back to the static terms line when %s" (5 rows). The other SignUp tests'
(`SignUp.client`, `SignUp.next`, `sign-in`, `worded`) fake servers now also answer the host and a TERMS text and wait for
it before clicking; no assertion changed.

Up-front tests (do not edit): `customer/sign-in/TermsText.state.test.tsx` (13: 12 red, 1 guard) and
`customer/sign-in/SignUp.terms-gate.test.tsx` (9: 8 red, 1 guard).

Note for the implementer: let `loadTerms` return the outcome (`{state:'ready', text} | {state:'missing'} |
{state:'failed'}`) instead of `null`, and keep the `ApiClient` in the component so a retry reuses it.

## 7. Round 3: admin, desk and staff screens

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-22 ICT. A read-only audit (sub-agent) of
> `screens/admin`, `operations`, `desk`, `desk-transaction` and `staff` on main `36abb596`, about 40 files in depth.
> The controller re-read R3-1, R3-2, R3-3 and R3-5 at `36abb596`. Rules as before: only bugs a person on screen can hit.

### 7.1 Bugs and root causes

| # | Screen | What the person sees | Root cause |
|---|---|---|---|
| R3-1 | Admin › Cash limits | After a branch switch whose `cash_limits` read fails, the banner and the input still show the **previous branch's** limit, and Save PUTs it under the new branch. A failed first read shows "no limit". | `CashLimits.tsx:94-97` sets only `refusal`; `applyData` runs only on success (`:91`); the branch effect (`:101-109`) never clears the form; Save (`:211`) is gated only on `busy()`. `CashLimitsSaveInput` carries no version, so the server cannot refuse the blind overwrite. |
| R3-2 | Admin › Cash transfer policy | A failed `cash_transfer_policy` read shows "No limit set yet" (`:175`) above empty inputs, and Save PUTs only what was typed: a **new org policy version that drops every limit the screen never showed**. | `CashPolicy.tsx:102-111` only sets `refusal`; Save (`:255`) is gated only on `busy()`; `TransferPolicySaveInput` carries no version. |
| R3-3 | Admin › Rates › rate policy card | When `me` fails and `rate_policy` answers after it, every policy input is disabled with no reason. | `Rates.tsx:178-182` puts the `me` failure in `policyError`; `readPolicy` success clears it (`:216`); they run in parallel (`:229`). |
| R3-4a | Admin › Cash, "show all" toggle | An error from the movements or shifts read vanishes, and the lists read "no movements" / "no shifts" over the emptied rows. | One `cashError` for three reads: `readCash` clears it on success (`Cash.tsx:77-82`, reached from `handleToggleShowAll` `:165-169`); movements and shifts set the same signal. |
| R3-4b | Admin › Cash › counts and transfers | After a failed `me` and a branch switch, the worded error disappears; every gated button stays disabled with only a tooltip until reload. | `CashFlows.tsx` `readMe` writes the text into `refusal` (`:93-97`), which `clearBranchState` wipes; `me` is never read again (`:181-184` reads it only while `held === null && err === null`) and there is no retry. |
| R3-5 | `/staff` header | The report link shows for `BranchDayOperate` (then the server refuses) and is hidden for `ShiftCloseCertify` / `OverviewRead` (which the server allows). | `StaffPage.tsx:145` checks `FinanceReconcile \|\| BranchDayOperate` (S-3 #575, before the server rule existed); the server rule is `staffDailyReport` = `AnyOf[ShiftCloseCertify, FinanceReconcile, OverviewRead]` (`api/crates/xc-auth/src/domain/policy/areas/staff_report.rs`). `StaffPage.test.tsx` B2 pins the old guess. |
| R3-6 | Desk B03 member tier badge | A network failure shows "Failed to fetch" in English; a refusal shows the server's raw message. | `TierBadge.tsx:51-53` uses `err.message` instead of `wordedError`. `TierBadge.test.tsx` row 3 pins the raw text. |
| R3-7 | Admin › Consent retention; Admin › Branches | The retention save toast reads "Save" instead of "Saved". While a branch profile loads, the screen reads "Saving…"; after a failed read, "No branches found" sits under the error. | `Consent.tsx:58` `toast(t('save'))`; `BranchesScreen.tsx:283` `loading() ? t('saving') : t('noBranches')`. |
| R3-8 | `/staff` shift card | A waiting cash count older than the 50 newest rows never shows on a busy branch. | `ShiftCard.tsx:66` reads `cash_counts` with no branch and no `status: 'WAITING'` filter; the admin screen sends both (`CashFlows.tsx:110-113`). |
| R3-9 | `/desk?kiosk=<id>` | If the kiosk request fails to open (expired, not found, network), the desk opens as a plain walk-in with no banner. | `desk/main.tsx:17` `void openEntry(store)`; `entries.ts:39-51` has no catch and the store has no entry-error state. **Owner decision pending** (below). |

### 7.2 Rules

| # | Rule |
|---|---|
| R3-1 | `CashLimits` keeps a `known` state: false at mount and **on every branch switch** (the switch also clears `current`, `typed` and `noLimit`, so no other branch's value is ever shown), true only after a successful read for the current branch. **The worded refusal alert stays exactly as today and stays the first `role="alert"`** (`CashLimits.tsx:164-166`); no second `role="alert"` is added. While a read has failed: the banner shows the new key `unreadable` (plain text, not an alert) in place of the limit or "no limit", and a `type="button"` retry (`retry`) beside it reads the same branch again. The "no limit" banner shows only after a successful read answered no limit. Save is `disabled` unless `known()` and not `busy()`, with `title` = the `unreadable` text after a failure, and `save` returns before sending unless `known()`. |
| R3-2 | `CashPolicy` keeps the same `known` state for the org policy read (it has no branch). **The worded refusal alert stays exactly as today and stays the first `role="alert"`** (`CashPolicy.tsx:183-185`, pinned by `worded.test.tsx` and `CashPolicy.branch-register.test.tsx` P5); no second `role="alert"` is added. A failed read shows the new key `unreadable` in the banner (plain text) in place of "No limit set yet", with a `type="button"` retry (`retry`) beside it that calls `readPolicy` again. "No limit set yet" shows only after a successful read answered none. Save is disabled unless `known()` (title as R3-1), and the submit handler refuses unless `known()`. The register read per branch is unchanged. |
| R3-3 | `Rates.tsx` gives `me` its own `meError` (never `policyError`), shows `<PermissionsUnavailable error={meError()} onRetry=… busy=…/>` whose retry re-reads `me`, and gives every policy control it disables for want of permission `title = permissionsUnavailableText(meError())` while `meError()` is set. Same pattern as `Pricing.tsx` since GME-P (`56bb8713`). `readPolicy` never touches `meError`. |
| R3-4a | `Cash.tsx` keeps one error per read: `cashError`, `movementsError`, `shiftsError`, each set and cleared only by its own read and shown above its own list (`role="alert"`). While a list's error is set, its empty text ("no movements" / "no shifts") is not shown. `isWriteDisabled` (`Cash.tsx:202`) checks **all three** errors, as today's single `cashError` does, so `Cash.recover.test.tsx` row 2 holds unchanged. |
| R3-4b | `CashFlows.tsx` `readMe` failure sets only `meError` (never `refusal`), shown with `<PermissionsUnavailable>` whose retry re-reads `me`. `clearBranchState` never clears `meError`, so a branch switch keeps it shown. Every gated button's `title` while `meError` is set is `permissionsUnavailableText(meError())`, as GM-2/GM-3 and R3-3. On a successful retry, the gates and the per-branch reads run as today. The `feat/r3-cash-errors` skeleton rewrites `CashFlows.rework.test.tsx` rows 1-6 (`:171-310`), which pin the bare worded text as the alert and the titles, to expect `permissionsUnavailableText`; the implementer does not touch that file. |
| R3-5 | The report link shows iff `me.permissions` holds any of `ShiftCloseCertify`, `FinanceReconcile`, `OverviewRead`, the server's `READERS`. The ticket rewrites `StaffPage.test.tsx` B2 to those rows (the old rows pinned a guess made before the server rule; the owner's rule is that the server's configurable permission decides). |
| R3-6 | `TierBadge` shows `wordedError(err)`; never `err.message`. `TierBadge.test.tsx` row 3 is rewritten by the ticket to expect the worded text. |
| R3-7 | Consent: a new `saved` key (th, en) for the success toast. Branches: a new `loading` key while the profile loads; no `noBranches` line while an error is shown. |
| R3-8 | `ShiftCard` sends `branch: props.ctx.me.branchId` and `filter: { status: 'WAITING' }` with its `cash_counts` read, as `CashFlows.tsx:110-113` does; it keeps the `api.view` call shape the existing ShiftCard tests fake. |
| R3-9 | Not ticketed until the owner decides. Proposal: a worded `role="alert"` banner "the kiosk request could not be opened" with a retry, and the desk stays usable as a walk-in. |

### 7.3 Tickets (File scopes disjoint)

| Ticket | File scope | Class | Base |
|---|---|---|---|
| R3-1 | `web/src/screens/admin/cash-limits/CashLimits.tsx` | logic | `feat/r3-cash-limits`, created with the skeleton keys and up-front tests before release |
| R3-2 | `web/src/screens/admin/cash-policy/CashPolicy.tsx` | logic | `feat/r3-cash-policy`, created with the skeleton keys and up-front tests before release |
| R3-3 | `web/src/screens/admin/rates/Rates.tsx`<br>`web/src/screens/admin/rates/Rates.me-error.test.tsx` | mechanical | main |
| R3-4 | `web/src/screens/admin/cash/Cash.tsx`<br>`web/src/screens/admin/cash/CashFlows.tsx` | logic | `feat/r3-cash-errors`, created with the up-front tests and the `CashFlows.rework.test.tsx` rewrite before release |
| R3-5 | `web/src/screens/staff/StaffPage.tsx`<br>`web/src/screens/staff/StaffPage.test.tsx` | mechanical | main |
| R3-6 | `web/src/screens/desk/loyalty/tier/TierBadge.tsx`<br>`web/src/screens/desk/loyalty/tier/TierBadge.test.tsx` | mechanical | main |
| R3-7 | `web/src/screens/admin/consent/Consent.tsx`<br>`web/src/screens/admin/consent/i18n.ts`<br>`web/src/screens/admin/consent/Consent.saved.test.tsx`<br>`web/src/screens/admin/branches/BranchesScreen.tsx`<br>`web/src/screens/admin/branches/i18n.ts`<br>`web/src/screens/admin/branches/BranchesScreen.loading.test.tsx` | mechanical | main |
| R3-8 | `web/src/screens/staff/shift/ShiftCard.tsx`<br>`web/src/screens/staff/shift/ShiftCard.waiting-filter.test.tsx` | mechanical | main |

### 7.4 Not a bug (round 3)

`DenominationGroups.tsx:169` and `Consent.tsx:118` (load failure as a toast, pinned); `FloorCard.tsx:173-175` (pinned by
#2721); `TransactionDetail.tsx:69-70` (pinned); `VerificationScreen.tsx:88-116` (a byte-identical copy of
`ApiClient.parseError`, no drift); `ApprovalsSection.tsx:66,134` (§11.9 rule 6); admin menus `needs:'all'` (OR-7.7);
the staff card gates (match `staff_floor.rs`, `cash.rs`, `branch_day.rs`); `AdminShell.tsx` branches read;
`Executive.tsx:221-238` (a failed poll reloads next tick); `AlertSettings.tsx` (sends `expectedVersion: null`, the
server checks it); `RateAlertSettings.tsx` (org-level); `Payments.tsx` (`canEdit` from the server); Reconciliation,
Approvals, Members, Loyalty, Customers, StartingPrices, BranchTablets, Notices, Campaigns and the desk customer,
phone-verify, kiosk-cancel, queue and desk-transaction paths (every failure worded). Pricing belongs to #4324's lane.

## 8. Round 4: kiosk settings, customer home, rate alerts, verification upload

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-22 ICT. A read-only audit (sub-agent) of the screens
> rounds 1–3 did not cover (`devices`, `documents`, `kiosk-currencies`, `kiosk-queue`, `login`, `receipt-sample`,
> `staff-guide`, `staff-report`, the remaining `customer/*` screens, and `shared/{live,notifications,queue,receipt,ui}`)
> on main `1ab96483`, about 30 candidates in depth. The controller re-read R4-1, R4-2 and R4-4.

### 8.1 Bugs and root causes

| # | Screen | What the person sees | Root cause |
|---|---|---|---|
| R4-1 | Admin `/kiosk-currencies` › queue machine `result_seconds` | The field shows `10` on open, whatever the branch has stored. Nothing reads the setting until the field gets focus or Read is pressed. After a failed read, or after a branch change, `10` (or the previous branch's value) stays and Save stays enabled, so Save, or the blur save, writes a value the screen never read over the branch's real setting. | `KioskCurrenciesScreen.tsx:318` starts the field at `'10'`; the branch effect (`:203-220`) neither reads the setting nor resets the field (it only clears the dirty flag, `:218`); a failed read (`:339-341`) leaves the value; Save (`:504-510`) and the `onChange` save (`:491`) are gated only on `savingSettings()`. |
| R4-2 | Customer `/app` home | A later branch-list poll failure (every 600 s; e.g. the phone wakes before the network) turns the branch picker into "couldn't load branches" **and** the healthy rate board into "couldn't load rates", whose Retry does nothing, for up to the branch poller's 10-minute backoff. A failed first branch read with no `?branch` leaves the rate section on "Loading rates…" for good. | `HomeScreen.tsx:173-179`: `branchesState` reports the poller's `'error'` even while it holds data (`PublicPoller.handleError` keeps the data, `public-poller.ts:166-173`). `:188-195`: `ratesState` is `'error'` whenever `branchesState` is, and `'loading'` whenever no branch is chosen. `:216-222`: the rate retry restarts only the rate poller. |
| R4-3 | Customer `/app/rate-alerts` | After a create, pause, resume or delete succeeds and the re-read fails: no word at all. A deleted alert stays with its buttons; a paused one still shows Pause; a new one is missing while the form clears, so the customer may create it again (it counts toward `alertLimitReached`). | `RateAlerts.tsx:199-203`: the `loadAlerts` catch words a failure only when the list is empty; the error box (`:471`) shows only for an empty list. Re-reads after writes: `:263`, `:281`, `:298`, `:315`. |
| R4-4 | Customer `/app/verification`, document upload | A 5xx, 401 or non-JSON answer (e.g. a proxy 413/502 page) shows the raw server text or the proxy page's text beside the upload button. | `customer/verification/VerificationScreen.tsx:63-66`: `formatUploadError` returns `err.message` for any `ApiError` whose message lacks the code, including `httpError` (whose message is the raw body, `shared/api/client.ts:92-95`), 5xx and 401. The §4 note on "VerificationScreen fallbacks" covers the staff screen's pinned cases, not this path. |

### 8.2 Rules

| # | Rule |
|---|---|
| R4-1 | The field has a `known` state: false at mount and **on every branch change**, true only after a settings read for the current branch succeeds. While not known, the field is **empty** (never `10`, never another branch's value), and Save, the `onChange` save and Enter send nothing (`disabled` / early return). A failed read shows its worded error as today and leaves it not known. **No read on mount**: focus and Read load the value, as today. A branch change also clears `resultSecondsInput`. Typing before a read is allowed but cannot be saved until a read succeeds. |
| R4-2 | The branch section shows its error state only when it has **no** data; after a later poll failure it keeps the list (as the customer `BranchesScreen` does, §8.4). Once a branch is chosen, the rate state and its retry come **only** from the rate poller, never from the branch state. A failed first branch read with no branch chosen shows the branch error, and the rate section **shows nothing** (no "Loading rates…", no rate error, no rate retry). |
| R4-3 | Every failed `loadAlerts` read sets a worded error. With alerts on screen, it shows as a separate non-blocking `<p role="alert" class="xc-alerts-reread-error">` line with a `type="button"` retry, and the list stays. `.xc-alerts-error` stays the empty-list box. The new line is **not** `.xc-form-error` and is not a direct child of `.xc-rate-alerts-screen`, because `worded.test.tsx:325-486` takes the first `.xc-rate-alerts-screen > .xc-form-error[role="alert"]` as the action error. The CSS rule for `.xc-alerts-reread-error` goes in `rate-alerts.css`, so the main-only `screen-look.test.ts` class ratchet passes. The error is cleared **only by a successful read**: the `setLoadError(null)` at the start of `loadAlerts` (`:187`) moves into the success branch. |
| R4-4 | `formatUploadError` returns `wordedError(err)` when `isServerFailure(err)` (5xx, `serverError`, `httpError`) or the status is 401. The server's 4xx sentences stay as today (pinned by `VerificationUploadErrors.test.tsx` rows `:59` and `:125`). |

### 8.3 Tickets (File scopes disjoint)

| Ticket | File scope | Class | Base |
|---|---|---|---|
| R4-1 | `web/src/screens/kiosk-currencies/KioskCurrenciesScreen.tsx` | logic | `feat/r4-result-seconds`, created before release with the up-front tests (`KioskCurrencies.result-seconds-known.test.tsx`) and the rewrite of `KioskCurrencies.result-seconds.test.tsx` "the save body" and "the refusal" (they type and save without a read) |
| R4-2 | `web/src/screens/customer/home/HomeScreen.tsx` | logic | `feat/r4-home-rates`, created before release with the up-front tests (`HomeScreen.later-failure.test.tsx`) |
| R4-3 | `web/src/screens/customer/rate-alerts/RateAlerts.tsx`<br>`web/src/screens/customer/rate-alerts/rate-alerts.css`<br>`web/src/screens/customer/rate-alerts/RateAlerts.reread-error.test.tsx` | mechanical | main |
| R4-4 | `web/src/screens/customer/verification/VerificationScreen.tsx`<br>`web/src/screens/customer/verification/VerificationUploadErrors.server.test.tsx` | mechanical | main |

### 8.4 Not a bug (round 4)

Devices: a branch switch remounts the screen (keyed `<Show>` in `devices/main.tsx`); "no transactions" over a failed
read is pinned with the worded message (`DevicesScreen.rework.test.tsx:164`); `failedViews` keeps each view's failure.
Documents `refusal.ts:9-10` (design D-A3/D-A4). Kiosk settings' 400 field sentence (pinned). Kiosk queue panel (offline
is worded in `QueueCard`). Login `home.ts` (§11.10). Staff report's sign-in link (v2 parity). `QueueDisplay` (§3 of
`web-public-get.md`), `RateDisplay` (offline after 30 s). Customer `BranchesScreen` keeps its list after a later poll
failure (deliberate, `:150`). `TakeNumber` (its own i18n text). Journey 403/404 as "no ticket" (pinned,
`journey.test.tsx:413`); journey without a branch (contract S5); `JourneyBookings` on 401. Assistant (no conversation id
in the contract). Notifications re-read after a mark (the optimistic state is right). Rate alerts keep data on a
background poll failure (pinned, `worded.test.tsx:497`). `CopyLink` (the server answers bad links 410). History has no
paging (no cursor in the contract). `PublicPoller` stops after a 404 (by design); `queue-store` stops on 401 (the
session-ended path, worded by `AppShell`). `VersionFooter`. Home preview and save-plan, Profile, Member, AnswerPreview:
worded.
