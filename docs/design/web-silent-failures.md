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
- **B3, the sign-up terms text** (`TermsText.tsx:37-39` returns null, so sign-up records TERMS consent to a text the person never saw). This waits on an **owner decision**: block sign-up until the text loads, or only warn.
- **B5, pricing `me` failure** (`Pricing.tsx:80-87`: `api.view('me')…catch(() => {})`, the #1719 pattern). Not fixed in this round. The admin pricing screen is under the pricing work of #4324 (its body names `web/src/screens/admin/pricing/TierEditor.tsx` in the same folder). No File scope at fa4163ab claims `Pricing.tsx` yet.
- **G-ME:** three staff screens pin a silent hide when `me` fails (`TransactionScreen.test.tsx:363`, `ReferralCodeField.permission.test.tsx:144`, `refunds.test.tsx:295`), against #1719's "worded, disabled with the reason". Fixing it needs a design of its own (a shared `meError` in the session store), and none exists at fa4163ab.
- **Already decided as "not a bug"** (so they are not re-audited): `BranchLive.tsx:33,45`, `save.ts:213,275`, `ReceiptFrame.tsx:47`, `tablet.ts:48`, `Shell.tsx:65`, `DeskBookingsScreen.tsx:212`, and the `UsersScreen` and `VerificationScreen` fallbacks pinned by their designs.
