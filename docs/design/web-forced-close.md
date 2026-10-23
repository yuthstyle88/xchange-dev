# Design: admin Cash, forced close of an abandoned shift (FC-FE)

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-23 ICT. It is the FE half of
> `docs/design/blueprint/cash-shift-transfer.md` §4.10 (A10, by claude/7ca80c, PR 4867).
> Owner: *"ทำ B ตามที่แนะนำ"*, recorded on #564 and confirmed in the BE lane's session. Ticketed only after the BE
> command (FC-2) is merged, because the route is the contract.

## 1. What the screen needs
- **The BE command.** §4.10 adds `countForcedClose`: `POST /api/shifts/{id}/forced-close-count`, body `{branch, lines, reason}`, answering `{id}`.
  - It needs `ShiftCloseCertify` (BRANCH_MANAGER).
  - It refuses the caller's own shift with 400 `validationFailed` (code `ownShiftUseCount`).
  - The reason must be 1–500 characters.
- **The second step is today's.** A second manager verifies the count through `countVerify`, with the counter not the verifier, and the FINANCE gate as for any close.
- **What the screen has today.** Admin Cash lists the branch's shifts with their status (`Cash.tsx:210`, `:518`) from `admin_shifts`. Its row type is `AdminShiftRow {id, status, openedAt, staff, note, difference, balances}`, and `staff` is the opener's **user id** (`admin_shifts.rs:216`, `:251`). `Cash(props)` has no `me` today: only the embedded `CashFlows` reads it. Its counts and transfers section shows WAITING counts and verifies them for `ShiftCloseCertify` holders (`CashFlows.tsx:35`, `:198`).

## 2. Rules
| # | Rule |
|---|---|
| FC-1 | `Cash.tsx` reads `me` itself (`api.view('me')`, like `CashFlows.tsx:95`; the existing Cash tests' fake servers already answer it). In the shifts list, an **OPEN** shift row shows a **Forced close** button (`data-action="forced-close"`) when `me.permissions` holds `ShiftCloseCertify` **and** `row.staff !== me.id`. The user's own shift never shows it. **If the `me` read fails** (G-ME, `web-silent-failures.md` §5), every OPEN row that is not provably the user's own shows the button **disabled**, with `title = permissionsUnavailableText(error)`, never hidden. |
| FC-2 | The button opens a dialog `ForcedCloseDialog`: the shift (its id, the opener's user id `staff`, opened at), one quantity input per denomination of the branch (the `denominations` view, read **when the dialog opens**, not on mount: `Cash.ca9.test.tsx:524` pins exactly one `denominations` read on mount), a required **reason** textarea (1-500 characters, counted), and Submit/Cancel. Submit is disabled until at least one line has a positive quantity and the reason is 1-500 characters after trim. |
| FC-3 | Submit sends `POST /api/shifts/{id}/forced-close-count` with `{branch, lines: CashLineInput[] (generated/cash.ts; only lines with a positive quantity), reason: reason.trim()}` through `keyedWrite` (one Idempotency-Key per attempt, as other cash writes). On success, close the dialog and show a worded toast (`forcedCloseSent`, "Sent; a second manager must verify"), then re-read the shifts and the counts, so the new WAITING count shows in the verify list. |
| FC-4 | A refusal shows worded through the screen's existing error mapping (`wordedError`), inside the dialog (`role="alert"`). The dialog stays open with its values. |
| FC-5 | In the WAITING counts list (CashFlows), a count is marked **Forced close** (a pill reusing the skeleton's `forcedClose` key) when `kind === 'CLOSE'` **and** its `countedBy` is not the `staff` of its shift. `CashFlows` already receives the shifts (`Cash.tsx:416`, `shifts={shifts}`). A normal count also carries a note, so the note alone can't single it out. The pill shows the count's note (the reason), so the second manager knows what they verify. Verify is unchanged; the server refuses the counter as verifier. |
| FC-6 | No new permission checks by role name (§11): only `me.permissions` and `openedBy`. |

## 3. Tickets (after FC-2 merges; File scopes disjoint)
| Ticket | File scope | Class |
|---|---|---|
| FC-FE-a | `web/src/screens/admin/cash/ForcedCloseDialog.tsx` (new), `web/src/screens/admin/cash/Cash.tsx`, `web/src/screens/admin/cash/i18n.ts`, the cash stylesheet, `web/src/screens/admin/cash/ForcedClose.test.tsx` (new) | logic |
| FC-FE-b | `web/src/screens/admin/cash/CashFlows.tsx`, `web/src/screens/admin/cash/CashFlows.forced-pill.test.tsx` (new) | mechanical |

FC-FE-a's up-front tests are written by the controller on a `feat/` base, with the i18n keys (`forcedClose`,
`forcedCloseReason`, `forcedCloseSent`, `forcedCloseTitle`) and CSS in the skeleton. FC-FE-b uses the FC-5 test (CLOSE and `countedBy` ≠ shift `staff`), re-checked against FC-2's merged contract before release, and reuses the `forcedClose` key (it does not touch `i18n.ts`).
