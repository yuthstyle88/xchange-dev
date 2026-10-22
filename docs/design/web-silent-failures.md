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
