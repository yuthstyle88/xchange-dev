# Design: customer screens key branches by id, not code

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-23 ICT. Found by the round-5 read-only audit (main
> `895234e1`); contract ruled by the xchange BE lane controller claude/7ca80c the same day. BE half: #4827.

## 1. Bug and root cause

`branches.code` is renamable: admin › Branches › profile save runs `UPDATE branches SET code = $2`
(`xc-rate` `branch_repository.rs:230`). The customer screens use `PublicBranchDto.code` as the branch's key and send it
as `branch`, but every view and command takes `branch` as a `BranchId`, matched against `branches.id`. Today the two
are equal, because they are set equal at creation (`branch_repository.rs:159`) and backfilled
(`20261011100000_organizations.sql:23`). After a code rename:

- the customer take-number, booking, planner, rate-alert, live-status and rate-board flows name a branch id that does
  not exist, and are refused or show nothing;
- `/app/bookings` keys its branch names and timezones by `code` (`Bookings.tsx:139-140`), but each booking row carries
  `branchId` (`MyBookingItemDto`), so the branch names and local times vanish.

## 2. Contract (BE lane ruling)

- `branches.id` is the stable key; `code` is a display label only.
- `PublicBranchDto` gains `id: string` as its first field (#4827, which regenerates
  `web/src/shared/api/public/public_branches.ts`).
- Making `code` immutable was **not** chosen, because it would remove an existing admin feature.

## 3. Rules (FE)

| # | Rule |
|---|---|
| CB-1 | Every customer screen keys a branch by `PublicBranchDto.id`: its selection state, `<option value>`, `find(...)` lookups, and `Map` keys. It sends `branch: id` to views and commands. |
| CB-2 | `code` and `name` are **display only**, e.g. `{name} ({code})`. No view, command, lookup or key uses `code`. **One exception, by contract:** the public read paths `/api/public/orgs/{org}/branches/{code}/…` (`rates`, `queue-rates`, `queue-display`; `api/src/presentation/http/public/*.rs`) are keyed by the branch's **current** code, and the server resolves it. A screen calls them with the `code` of the branch entry it matched in the public branch list, never with a code it kept from a URL or from an earlier read. |
| CB-3 | **URLs carry the id.** Links the screens build (`/rates?branch=`, `/app/book?branch=` and the like) put the id in `?branch=`. |
| CB-4 | **Reading `?branch=` accepts an id or a code**: match `id` first, then `code`, and keep the matched branch's `id` from then on. Links and TV-board URLs that were already printed carry codes; they keep working until that code is renamed. This is a reader rule for URLs only; requests never send a code. |
| CB-5 | Components that take a `branch` prop (`BranchLive`, `TakeNumber`) receive the id. Their doc comment says "the branch id". |
| CB-6 | **The rate board** (`RateDisplay`, `BoardShell`, `BookingQr`) resolves its branch from the public branch list by `?branch=` (id or code, CB-4), and waits for the list before its first rates read. It calls the public rates path with the matched entry's current `code` (CB-2), and its booking QR, which opens `/app/queue?branch=` (`BookingQr.tsx:17-18`), prints the matched entry's **current `code`**, because `/app/queue` is code-keyed (CB-7; the CB-2 exception). That QR is the one exception to CB-3. |
| CB-7 | **The `/app/queue` family stays code-keyed, as it is** (`queue-state.ts`, `queue-tablet.ts`, `QueueDisplay.tsx`, the admin `BranchTablets.tsx:104` link). It reads `?branch=<code>`, resolves it through the server's code-keyed `queue-rates` to `branchId` (`queue-state.ts:146-147`), and sends that id in every command (`:171`). Nothing there changes, and no ticket touches it. |

## 4. Tickets (File scopes disjoint; all blocked on #4827 merging)

| Ticket | File scope | Class |
|---|---|---|
| CB-Book | `web/src/screens/customer/book/Book.tsx` + a new test file in its folder | mechanical |
| CB-Bookings | `web/src/screens/customer/bookings/Bookings.tsx` + a new test file | mechanical |
| CB-Branches | `web/src/screens/customer/branches/BranchesScreen.tsx`, `BranchLive.tsx`, `TakeNumber.tsx` + a new test file | mechanical |
| CB-Home | `web/src/screens/customer/home/HomeScreen.tsx` + a new test file | mechanical |
| CB-Alerts | `web/src/screens/customer/rate-alerts/RateAlerts.tsx` + a new test file | mechanical |
| CB-Board | `web/src/screens/customer/rate-display/RateDisplay.tsx`, `board/BoardShell.tsx`, `BookingQr.tsx` (not under `board/`) + a new test file | mechanical |

Each ticket's tests use a fixture branch whose `code` differs from its `id`. They prove that requests carry the id,
that labels show the code, and (where the screen reads `?branch=`) that both an id and a code in the URL resolve.
Existing tests that pin `?branch=<code>` or a request `branch: <code>` are listed in each ticket's scope, where the
fixture makes them differ.

## 5. Added after the CB-Home and CB-Board blocks (claude/e8020026, 2026-10-23 ICT)

| # | Rule |
|---|---|
| CB-8 | **Home with `?branch=` set and a failed first branch-list read:** the branch cannot be resolved (CB-2 needs the matched entry's current code), so the rate section **shows nothing** (no rates, rate error, rate Retry or "Loading rates…"), the same as `web-silent-failures.md` §8.2 R4-2's no-branch case. The branch error and its Retry show; when the Retry loads the list, the branch resolves and the rates read runs. This **supersedes** the `?branch` row R4-2 implied and #4783 pinned (row (e) of `HomeScreen.later-failure.test.tsx`); #4833 rewrites that row. |
| CB-9 | **Rate board tests:** the rates read now waits one more async step (the branch list, CB-6). Tests that settle with a fixed tick count get more ticks, or await the list request. No assertion changes (#4835). |
