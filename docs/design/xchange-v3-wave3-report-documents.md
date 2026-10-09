# XCHANGE v3 — Wave 3: the documents pages and the staff report

Part of Phase 2 (`xchange-v3-web.md`). That document's §6 lists two Wave 3 rows that this file designs:

| Path | Today | Section |
|---|---|---|
| `/documents`, `/staff/documents` | HTML that v3 writes itself (`api/src/presentation/http/documents.rs`) | §A (this PR) |
| `/staff/report` | v2's built `staff-report.js` (React, 202,423 B), reading `adminSnapshot` | §B, in `xchange-v3-wave3-staff-report.md` (PR 533) |

V4 (§7 of `xchange-v3-web.md`) needs no new design. Its ticket is cut after every V3 ticket has a number.

**Who designs what in Wave 3** (three ticket-issuing sessions, agreed 2026-10-09):
- `xchange-v3-wave3-desk.md` covers `/desk` and `/staff/desk`.
- `xchange-v3-wave3-staff.md` covers `/staff` and the shared staff views: operations, requests, attendance and holds.
- This file covers `/documents`, `/staff/documents` and `/staff/report`.

No two of these files define the same view. §B reads the staff views; it defines only what the report computes.

Measured at `origin/main` `9f05f83` (2026-10-09).

---

## §A. The documents pages

### A.0 The Blueprint decides the rules (owner, 2026-10-09)

The owner, in session claude/e5ab6c on 2026-10-09: *"พิมพ์ เค้าว่าอย่างไรให้ ยึดตามนั้น"* and *"เพราะ พิมพ์เขียน ระบุไว้
ก็ยึดตามเอกสาร"*. On #564: Blueprint logic is fixed, and only the look may change. This page keeps v2's **look**
(A.1). Its **rules** are the Blueprint's wherever the Blueprint states one:

| Rule | Blueprint | § | Designed in | Effect here |
|---|---|---|---|---|
| Identity documents are stored apart, in a private object store, and opened only through a **short-lived authorized download** | *"เก็บไฟล์แยก private object store; short-lived authorized download; ไม่คืนภาพเต็มใน list/marketing export"* | 16 `VerificationDocument` | #557 | the passport `href` in `DocumentsLink` is the short-lived link #557 defines, not the session-wide `/files/<key>` |
| Marketing and Partner never open identity documents | *"ไม่เปิดสิทธิเอกสารยืนยันตัวตน"*; *"Marketing/Partner ดาวน์โหลดเอกสารไม่ได้"* | 02, 18 | #550, #557 | the views' reader set follows #550's roles; no role check is written in the views |
| A production receipt comes only from a completed Transaction, and is never overwritten (`Receipt.revision`) | *"production receipt ออกจาก completed Transaction เท่านั้น"*; *"ใบเสร็จไม่ overwrite"* | 14, 07 | #551 | the receipt link points at the receipt of the exchange's completed Transaction; **which revision** it shows is #551's design to state (the Blueprint gives `Receipt.revision` and "never overwritten", not the choice) |
| No full PII in any log | 07 | 07 | #549 (PR 570 §5.5) | the views write no audit lines with document contents |

Until #557 and #551 merge, the rows of A.1 describe v3 **as measured** (session file links, v2's labels).
- The V3-documents ticket is cut against the merged designs of #550, #551 and #557, using their names. Where they say
  otherwise, they win over A.1.
- The A.1 rows that this changes are marked **[Blueprint]**: steps 3–7, 16–23 and 28–32 (the links), and 22–23 and
  37–38 (who may read).

### A.1 What v2 does: the behaviour contract (W9). This is the look; the rules are A.0's

v2's behaviour is already recorded. `api/tests/fixtures/documents/v2_document_cases.json` holds 39 steps, and
`api/tests/documents_test.rs:204` (`the_two_pages_answer_as_v2_answered`) replays them against today's
`http/documents.rs`. The table below groups those steps. **v3** is what the screen and its view must do; every row
becomes a test (§A.6).

| Steps | Who | Request | v2 answers | v3 answers |
|---|---|---|---|---|
| 0, 1 | nobody; a first-password employee | `GET /documents?ref=…` | 303 → `/login` | the same: gate `signed-in` (`screens.rs:130`); a first-password employee has no session |
| 2, 27 | anyone | `POST /documents`, `POST /staff/documents` | 404 | the same: `ScreenTable` answers a non-GET with `plain_404` (`screens.rs:120`) |
| 3–7, 16–18, 20–23 **[Blueprint]** | owner (any branch via `?branch=`, empty = `MAIN`); any employee (own branch; `?branch=` ignored), incl. RATES and SYSTEM | `GET /documents?ref=<own>` | 200: heading `เอกสารรายการ`, `<ref> · ใบเสร็จและพาสปอร์ตผูกกับรหัสออเดอร์เดียวกัน`, one item per file: label — `เปิดดู` (new tab) · `ดาวน์โหลด <ref>_<KIND>.<ext>`, link back `/?branch=<b>#transactions` | the same items, from the view `documents`. The owner names the branch through `?branch=`; an employee always gets their own branch (`Policy::branch`) |
| 6 | owner | a transfer | three items: passport, payment proof, receipt (in that order) | the same order, which is the order `DocumentsLookup::find` returns |
| 8, 9, 12–15, 19 | owner or employee | another branch's ref, the wrong branch named, an unknown ref, no ref, empty ref, a demo exchange | 404 `text/plain` `ไม่พบเอกสารรายการ` | the page loads (200), the view answers 404 `documentsNotFound`, field `ref`, sentence `ไม่พบเอกสารรายการ`, and the screen shows that sentence where the list would be. **Known difference** (D-A3) |
| 10 | owner | `?branch=NOWHERE` | 404 `ไม่พบสาขาที่เลือก กรุณารีเฟรช` | the view core's `require_branch` (`view.rs:357`) answers 400 `noSuchBranch` with the **same sentence**, `ไม่พบสาขาที่เลือก กรุณารีเฟรช`, and the screen shows it. **Known difference** for the status only (D-A3) |
| 11 | owner | `?branch=MAIN_1` (cannot be a branch id) | 404 `สาขาไม่ถูกต้อง`, before the branch is looked for (#139, `documents_query_test.rs:68`) | the view core refuses before loading, **byte-identical to today**: 400 `invalidRequest`, no field (`view.rs:140-147`). The screen maps that refusal to `สาขาไม่ถูกต้อง`, v2's sentence verbatim (W6, §A.7). **Known difference** for the status only (D-A3) |
| #139 | owner, employee | `?ref=A&ref=B`, `?branch=X&branch=Y` | the **first** of a repeated name counts (`documents_query_test.rs:100`, `:147`) | the same: the screen reads `URLSearchParams.get`, which returns the first, and passes that one value to the view. The view core also keeps the first one (`view.rs:136-137`) |
| #139 | — | a file whose bytes are gone; a file an employee stored | the type comes from the record, and both are found (`documents_query_test.rs:169`, `:199`) | the same. `DocumentsLookup` is unchanged. The page assertions of those two tests move to **V** (§A.7). The lookup half of `:199` (`:229-238`, `lookup.find`) stays untouched |
| 24–26 | nobody; the owner; a first-password employee | `GET /staff/documents?ref=…` | 302 → `/staff` | the page loads, the view `staff_documents` refuses (no staff session), and the screen calls `location.replace('/staff')`. **Known difference** (D-A4) |
| 28–32 **[Blueprint]** | FRONT, MANAGER, FRONT of BR-2 (owner also signed in: still the employee) | `GET /staff/documents?ref=<own>` | 200: heading = the ref, one `<p><a href="/api/staff/file/<key>">label</a></p>` per file, back link `/staff` | the same links, from the view `staff_documents`: `href` already rewritten to `/api/staff/file/…`. No download name: v2's staff page has none |
| 33–36 | FRONT | another branch's ref (`?branch=` ignored), an unknown ref, no ref, a demo exchange | 302 → `/staff` | the view answers 404 `documentsNotFound`, and the screen calls `location.replace('/staff')` (D-A4) |
| 37, 38 **[Blueprint]** | RATES, SYSTEM | `GET /staff/documents?ref=…` | 302 → `/staff` | the view refuses through `Policy::allow(StaffRpc, documents)` (403), and the screen calls `location.replace('/staff')` (D-A4) |

### A.2 Decisions

- **D-A1. Two views, one per path, over one core** (`xchange-v3-web.md` §4.4: *"When one v2 action served both
  screens, they get two views, one per path"*; ♻️ rule). `documents` inherits `(UserRoute, "documents")`, and
  `staff_documents` inherits `(StaffRpc, "documents")`. The two differ only in their name, which entry path they
  inherit, and how a file's link is written. So one generic view `DocumentsBranchView<A: DocumentsAudience>` holds
  everything else.
- **D-A2. One screen, two entries.** `DocumentsScreen` takes an `audience` prop. Two HTML entries declare the two
  paths. The parts that differ live in one record per audience (`audiences.ts`): the view name, the back link,
  whether download names are shown, and what a refusal does.
- **D-A3. A refusal on `/documents` is a sentence on the page, not a 404 page.** A v3 screen is a page that loads and
  then asks its view (§3.3). The view's status is the same refusal v2 gave, but the HTML page itself is 200. The
  sentence is the view's (`documentsNotFound`). A branch that does not exist, or a malformed branch, gets the view
  core's sentence, which differs from v2's text. Both differences go into `xchange-v3-known-differences.md` in the
  ticket's PR.
- **D-A4. `/staff/documents` sends every refusal back to `/staff` from the browser** (`location.replace`), because
  v2's server sent a 302. No new gate is added. A `staff` gate would mean editing `api/src/presentation/screens.rs`
  and `api/build_support/screens.rs`, and #472 (V2-switch) holds `screens.rs`. The 200-then-redirect replacing the
  302 is a known-differences row.
- **D-A5. The file name and the kind of a document live in the application layer.** Today `ending()` is written
  twice: `http/documents.rs:122` and `http/v3/views/transaction_documents.rs:39`. The label-to-kind match is written
  twice as well. `DocumentLink` gains `kind()` and `file_name(document_ref)`, and the new view uses them.
  `transaction_documents.rs` belongs to V2-parts (#476, merged). Re-pointing it at the shared functions is a
  follow-up ticket, not this one (♻️: an existing caller keeps its contract and is re-pointed later).
- **D-A6. The ticket retires `http/documents.rs`.** `http::configure` is registered **before** `ScreenTable`
  (`bootstrap/mod.rs:92-93`), so while `documents.rs` is in place it answers both paths and the screen is never served.
  The ticket deletes the file and its lines in `http/mod.rs:9` and `:33`, and moves the page tests to the views. The 39-step
  fixture **stays**, and a new replay asserts every step against the views (§A.6). No step is dropped: each
  either keeps v2's answer or appears in known differences.
- **D-A6a. Retiring provided tests: ruled by the lane controller on 2026-10-09** (PR 530,
  issuecomment-6072805835). `documents_test.rs` and `documents_query_test.rs` each begin *"do not edit, delete or
  weaken"* (tickets T8b and #139), and their **page** tests assert server-written HTML. The ruling approves moving them,
  on three conditions:
  - every old assertion maps 1:1 to a new test (§A.8);
  - the 39-step fixture file is kept, unchanged;
  - the lookup tests stay untouched (`documents_test.rs:42`, `:56`, `:140`, and the `lookup.find` half of
    `documents_query_test.rs:199`, i.e. `:229-238`).

  The reviewer of the ticket's PR checks §A.8 against the old files line by line. The sentence `สาขาไม่ถูกต้อง` stays
  (§A.7).
- **D-A6b. Headers.** v2's pages carried `no-store` (owner) and `private, no-store` (staff). The v3 HTML shell holds no
  document data and is sent `no-cache` (`screens.rs:166-170`). The data travels in the view answer, which is
  `no-store` (`view.rs:459`, `:542`), so no document link is cached.
- **D-A6c. Not ported: v2's `ภาพรายละเอียดใบเสร็จ` link.** v2 archived the receipt image on first open
  (`src/devices.mjs:240-243` in the v2 copy). v3's page never had that link, and `DocumentsLookup` returns three
  kinds. This screen keeps v3's three kinds. Porting the link would be its own ticket, if the owner asks for it.
- **D-A6d. RATES and SYSTEM on `/documents`** (**[Blueprint]**: superseded by A.0's roles once #550 merges). Today they see the list (steps 22–23), but `/files/` refuses them the file
  (`policy.rs` `allow_file`). That is v2 parity, and W8 keeps it.
- **D-A6e. No rate is read, written or computed here.** The owner changed the rate model on 2026-10-09 ("follow the
  Blueprint fully", PR 527 issuecomment-6073063976): THB per `unit_size`, scale 8, draft → approve → publish. That
  change does not touch §A. The documents pages list an exchange's files (passport, payment proof, receipt) and
  nothing else. `DocumentsData` carries no amount and no rate. The word "RATES" in this file is the employee role,
  not a rate.
- **D-A7. Texts** (W10). The screen's own texts go in `web/src/screens/documents/i18n.ts`, namespace
  `documents.*`, in Thai and English. The label of a file comes from its `kind`
  (`documents.kind.passport | paymentProof | receipt`), not from v2's Thai label. The Thai values are v2's words
  verbatim.

### A.3 Class diagram

```mermaid
classDiagram
  class DocumentLink {
    +label: String
    +url: String
    +mime: String
    +kind() DocumentKind
    +file_name(document_ref: &str) String
  }
  class DocumentKind {
    <<enum>>
    Passport
    PaymentProof
    Receipt
  }
  class DocumentsLookup {
    +find(branch: &BranchId, reference: &str) AppResult~Documents~
  }
  class DocumentsAudience {
    <<trait>>
    +NAME: &'static str
    +INHERITS: (EntryPath, &'static str)
    +SHOW_DOWNLOAD: bool
    +href(url: &str) String
  }
  class OwnerDocuments
  class StaffDocuments
  class DocumentsBranchView~A~ {
    -lookup: DocumentsLookup
    +load(branch, who, filter) AppResult~DocumentsData~
  }
  class DocumentsFilter { +ref: String }
  class DocumentsData { +ref: String; +links: Vec~DocumentsLink~ }
  class DocumentsLink { +kind: DocumentsLinkKind; +href: String; +download: Option~String~ }

  DocumentsAudience <|.. OwnerDocuments
  DocumentsAudience <|.. StaffDocuments
  DocumentsBranchView ..|> BranchView
  DocumentsBranchView --> DocumentsLookup : find
  DocumentsBranchView --> DocumentsAudience : NAME, INHERITS, href
  DocumentsLookup --> DocumentLink : returns
  DocumentLink --> DocumentKind
  DocumentsBranchView --> DocumentsData : answers

  class DocumentsScreen {
    +props: api, audience: Audience, reference, branch
  }
  class Audience {
    <<record>>
    +view: 'documents' | 'staff_documents'
    +backHref(branch) string
    +showDownload: boolean
    +onRefusal(err: ApiError) void
  }
  DocumentsScreen --> Audience
  DocumentsScreen --> ApiClient : view(audience.view, filter: ref)
```

| Class / item | File | New? | Provides to | Uses |
|---|---|---|---|---|
| `DocumentKind`, `DocumentLink::kind`, `DocumentLink::file_name` | `api/src/application/workflows/documents.rs` | adds to it | the view | the label constants `PASSPORT`, `PAYMENT_PROOF`, `RECEIPT` (`:16-18`) |
| `DocumentsAudience`, `OwnerDocuments`, `StaffDocuments`, `DocumentsBranchView<A>`, `register` (registers both), `export_types` | `api/src/presentation/http/v3/views/documents.rs` | new | `ViewEndpoint` | `DocumentsLookup`; `compat::staff_view::rewrite_urls` for the staff `href` (`staff_view.rs:156`) |
| `DocumentsFilter` (`ViewFilter::check` refuses nothing: an empty ref is v2's 404, not a 400), `DocumentsData`, `DocumentsLink`, `DocumentsLinkKind` | `api/src/presentation/dto/v3/documents.rs` | new | the view, the generated TS | `V3Error` |
| generated `DocumentsData` etc. | `web/src/shared/api/generated/documents.ts` | generated | the screen | — |
| `DocumentsScreen` | `web/src/screens/documents/DocumentsScreen.tsx` | new | both entries | `ApiClient.view`, `t()` |
| `OWNER`, `STAFF` (`Audience`) | `web/src/screens/documents/audiences.ts` | new | both entries | — |
| entry scripts | `web/src/screens/documents/owner.tsx`, `web/src/screens/documents/staff.tsx` | new | the HTML entries | `DocumentsScreen`, `audiences.ts` |
| texts | `web/src/screens/documents/i18n.ts` | new | the screen | `shared/i18n` |
| `refusalText` | `web/src/screens/documents/refusal.ts` | new | the screen | `ApiError` `status`, `code`, `field` (§A.7) |
| HTML entries | `web/pages/documents.html` (`xc-path /documents`, `xc-gate signed-in`), `web/pages/staff-documents.html` (`xc-path /staff/documents`, `xc-gate anyone`) | new | `ScreenTable` | — |
| retired | `api/src/presentation/http/documents.rs` (deleted), its two lines in `api/src/presentation/http/mod.rs` (`:9` `pub mod documents;`, `:33` `documents::configure`) | deleted | — | — |

The view `documents` answers `404 documentsNotFound` (field `ref`, sentence `NO_DOCUMENTS`) when the lookup
answers `AppError::Domain(NO_DOCUMENTS)`. It does **not** answer an empty list the way `transaction_documents` does
(`transaction_documents.rs:96-100`), because v2's page was a 404. Any other error goes through `V3Error::from`.

`DocumentsLink.download` is `Some(file_name)` for `OwnerDocuments` and `None` for `StaffDocuments`
(`SHOW_DOWNLOAD`). The owner's file name uses v2's `<ref>_<PASSPORT|PAYMENT|RECEIPT_FULL>.<ext>`
(`documents.rs:143-147`).

### A.4 Who may read it

The two views inherit from `xchange-v3-web.md` §4.4, and the role table is not edited. `UserRoute documents` admits
the owner and every signed-in employee (steps 17–23). `StaffRpc documents` admits staff except RATES and SYSTEM
(steps 37–38), and `authenticate_and_authorize` reads only the staff session for `StaffRpc` (`view.rs:340`). So step
30, where the owner and FRONT are signed in together, is answered for FRONT.

### A.5 Ticket

One ticket, **V3-documents**. It owns both paths, because the server core, the screen and the retirement are one
piece of work: splitting it would leave a path served by neither the old page nor the new one.

File scope: every file in the §A.3 table, plus `api/tests/documents_test.rs` and `api/tests/documents_query_test.rs` (only the page tests leave them, per D-A6a and §A.8), `api/tests/v3_documents_screens_test.rs` (new),
`api/tests/v3_documents_views_test.rs` (new), `web/src/screens/documents/DocumentsScreen.test.tsx` (new),
`web/src/screens/documents/entries.test.tsx` (new), and
`docs/design/xchange-v3-known-differences.md` (rows only).

**Start dependency (A.0):** the merged designs of #550 (roles), #551 (Receipt) and #557 (VerificationDocument and its short-lived download). The ticket is cut after they merge, not before.

Ordering: the ticket merges after #472 (V2-switch), because the owner's back link points at `/#transactions` on the
v3 admin. It needs nothing #472 delivers to start, so it is **not** `Blocked by`. It shares no file with the desk or
staff designs.

### A.6 Tests the ticket author writes first (🧪)

On the ticket's branch `feat/v3-documents-<n>`, before dispatch:

- `api/tests/v3_documents_views_test.rs` replays **all 39 fixture steps** against the two views (`GET
  /api/v3/views/documents?…` / `staff_documents?…`, with the step's sessions). For each step it asserts the status,
  plus either the links (kind, href, download, order) or the refusal `code` and `field`. Steps 2 and 27 (`POST`)
  assert the screen table's 404. To prove the test can go red, it must fail on `main` because the views do not exist,
  and fail against each mutant: `StaffDocuments::href` returning the owner URL, the 404 returned as an empty list, and
  `INHERITS` of `staff_documents` set to `UserRoute` (which turns steps 37–38 into 200), and `refusalText` showing the raw sentence for a no-field `invalidRequest` (the bad-branch rows of **S** go red). §A.7 lists every test these files must hold.
- `web/src/screens/documents/DocumentsScreen.test.tsx` uses a fake `ApiClient` and covers:
  - the owner's list: label by kind, `เปิดดู` with `target=_blank` and `rel=noopener`, the download name, and the back
    link `/?branch=BR-2#transactions` with the branch encoded;
  - the staff list: no download link, back link `/staff`;
  - the `/documents` refusal, which shows the view's sentence;
  - each `/staff/documents` refusal (401, 403 and 404), which calls the injected `replace('/staff')`;
  - TH and EN both rendered from `documents.*` keys.
- `web/src/screens/documents/entries.test.tsx` checks the two HTML entries' `xc-path` and `xc-gate`.

### A.7 A bad branch keeps v2's sentence, and the view core stays byte-identical

v2 answers a branch name that cannot be a branch id with `สาขาไม่ถูกต้อง`. That is v2's wording in
`xchange-v2-source@4d0bdbb` `src/devices.mjs:72`, and v3 keeps the same sentence in at least six places (e.g.
`compat/rpc.rs:79`, `http/bookings.rs:47`, `files/file_service.rs:15`). The view core refuses that name in
`parse_query` (`api/src/presentation/http/v3/view.rs:140-147`) with 400, `invalidRequest`, no field, `คำขอไม่ถูกต้อง`.

**Two provided tests pin that refusal exactly, so the core does not change at all:**
- `api/tests/v3_view_test.rs:684` (via `is_refusal` `:423`);
- `api/tests/v3_view_unknown_branch_test.rs:810`, which pins the whole body
  `{"error":{"code":"invalidRequest","message":"คำขอไม่ถูกต้อง"}}` for `?branch=a%2Fb` (`:824`).

Both are on the ticket's must-stay-green list. `view.rs` is **not** in the ticket's scope.

**The mechanism is screen-only**, `web/src/screens/documents/refusal.ts`, **[required]**:

```ts
export function refusalText(err: ApiError, t: (key: string) => string): string
```

It returns `t('documents.badBranch')` when `err.status === 400 && err.code === 'invalidRequest' && err.field ===
undefined`, and `err.message` otherwise. `documents.badBranch` is th `สาขาไม่ถูกต้อง` (v2 verbatim) and en
`Invalid branch`.

**Why a no-field `invalidRequest` from these two views can only be a bad branch.** The screen is the only caller, and
it builds the request itself:
- one `branch`, which it takes from `URLSearchParams.get`;
- the filter `ref`. `DocumentsFilter::check` refuses nothing, and a filter refusal would carry `field: "ref"` anyway.
- no `since`, `before` or `limit`, because the screen reads once and never pages.

`ApiClient` encodes the query (`client.ts:73-106`), so the "unparsable query" refusal at `view.rs:126-133` cannot
come from it either. A test in **S** pins the mapping. A test in **V** pins what the view really answers for
`?branch=MAIN_1`: 400, `invalidRequest`, no field.

The rule is the same one `/documents` used: `pages::is_valid_branch` = `BranchId::is_well_formed` (`pages.rs:507`), the
same function `parse_branch` calls at `view.rs:322`. So the seven malformed names of `documents_query_test.rs:68` stay
malformed, and the three well-formed unknown names stay "no such branch".

**One case differs from v2: an employee on `/documents?branch=MAIN_1`.**
- v2 ignored any name an employee gave (`documents.rs:175`, `actor.branch_id()` first).
- The view core parses the name before `Policy::branch`, so v3 refuses it, and the screen shows `สาขาไม่ถูกต้อง`.
- No recorded step covers it, and a desk link never carries a malformed branch (`compat/actions/save.rs:105`).
- This is a known-differences row in the ticket's PR.

### A.8 Old tests → new tests (1:1)

The new files are:
- **V** = `api/tests/v3_documents_views_test.rs`
- **S** = `web/src/screens/documents/DocumentsScreen.test.tsx`
- **E** = `web/src/screens/documents/entries.test.tsx`
- **T** = `api/tests/v3_documents_screens_test.rs`, which asserts the page answers through `bootstrap::configure` with a
  built `web/dist`, the way `v3_admin_branches_view_test.rs:707` does.

| Old assertion | Where it goes | Kept / changed |
|---|---|---|
| `documents_test.rs:204` replays the 39 steps | **V** `the_39_steps_answer_as_section_a1_says`: same fixture, one expectation per step from §A.1; steps 2 and 27 in **T** | kept per step; changes are only §A.1's D-A3/D-A4 rows |
| `documents_test.rs:214`: POST, PUT, DELETE, PATCH on both paths → `(404, "Not found")` | **T** `a_wrong_method_is_the_screen_tables_404`: the same 8 requests, the same `(404, "Not found")` (`pages::plain_404`, `pages.rs:732`) | kept |
| `documents_test.rs:237`: no session reader, `/documents` → 303 `https://desk.example/login` | **T** `without_a_session_reader_documents_is_sent_to_login`: gate `signed-in` → `pages::to_login` (`pages.rs:600`), same status and Location | kept |
| `documents_test.rs:237`: no session reader, `/staff/documents` → 302 `/staff` | **V**: `staff_documents` with no session reader → 401. **S**: a 401 calls `replace('/staff')` | changed (D-A4): 200 shell, then a browser redirect |
| `documents_test.rs:263`: a page names only the asked exchange (owner, FRONT, staff path) | **V** `an_answer_names_only_the_exchange_asked_for`: the answer's `ref` and every `href`/`download` hold only `cash`'s keys, none of `transfer`/`other`'s, for the same three (who, path) | kept |
| `documents_test.rs:263`: the staff page never links `/files/`, and a transfer has exactly 3 `/api/staff/file/` links | **V**: every `staff_documents` `href` starts `/api/staff/file/`, none `/files/`, and a transfer has 3 links | kept |
| `documents_test.rs:315`: DB down, `/documents` as owner and as FRONT → 500 `{"error":"เซิร์ฟเวอร์ขัดข้อง"}` | **V** `when_the_database_is_down_the_view_answers_500`: 500, code `serverError`, sentence `เซิร์ฟเวอร์ขัดข้อง`, nothing of the cause. **S**: the sentence is shown | changed: the body shape is V3Error's, the sentence is the same |
| `documents_test.rs:345-350`: DB down, nobody signed in → 303 / 302 | **T**: `/documents` 303 needs no DB (gate). `/staff/documents` → 200 shell (D-A4) | 303 kept; 302 changed (D-A4) |
| `documents_query_test.rs:48`: the six measured branch rows | **V** `the_six_measured_branch_rows`: `MAIN_1` → 400 `invalidRequest`, no field; `B-nowhere` → `noSuchBranch` + `NO_SUCH_BRANCH`; `MAIN` → `documentsNotFound` + `NO_DOCUMENTS`; the three repeated pairs → the first name decides (`view.rs:136-137`). **S**: that refusal shows `สาขาไม่ถูกต้อง` | sentences kept; status 404 → 400 for the two branch rows (D-A3) |
| `documents_query_test.rs:68`: 7 malformed names → `สาขาไม่ถูกต้อง`; 3 well-formed unknown → `NO_SUCH_BRANCH`; empty → `MAIN` 200 | **V** the same 11 names: 7 → 400 `invalidRequest` with no field, 3 → `noSuchBranch`, empty → 200 for `MAIN`. **S**: the no-field `invalidRequest` → `สาขาไม่ถูกต้อง` | kept (§A.7) |
| `documents_query_test.rs:100`: repeated `ref`, the first counts on both pages; a first `ref` nobody has → not found (owner) / 302 (staff) | **S** `the_first_ref_is_the_one_asked_for`: from `?ref=A&ref=B` the screen asks the view for `A` only, on both audiences. **V** the single-ref answers. **S** `XC-NOPE-1` first → owner shows `NO_DOCUMENTS`, staff calls `replace('/staff')` | kept: the screen sends one `ref`, because the view's filter takes one value |
| `documents_query_test.rs:147`: repeated `branch`, the first counts | **S**: from `?branch=BR-2&branch=MAIN` the screen names `BR-2`. **V**: `branch=BR-2&branch=MAIN` → found; `branch=MAIN&branch=BR-2` → `documentsNotFound` | kept |
| `documents_query_test.rs:169` (whole test, `:174-191`): the owner's download names keep `.png`/`.pdf` after the stored bytes are removed | **V** `a_file_whose_bytes_are_gone_keeps_its_type`: the `documents` view's `download` for PASSPORT ends `.png` and for PAYMENT `.pdf`, before and after removing the bytes, with the exact names `<ref>_PASSPORT.png` / `<ref>_PAYMENT.pdf` | kept |
| `documents_query_test.rs:199`, page half (`:240-262`): both pages list an employee-saved exchange's passport and slip, and the owner's names end `.jpg`/`.pdf` | **V** `the_files_an_employee_saved_are_in_both_views`: `documents` (owner) and `staff_documents` (FRONT) both hold the passport and slip keys, and the owner's `download` names end `.jpg`/`.pdf` | kept |
| `documents_query_test.rs:199`, lookup half (`:229-238`, `lookup.find` → mimes `image/jpeg, application/pdf, text/html`) | stays in the same test, whose page half (`:240-262`) is removed | untouched (condition c) |

Nothing is dropped. The two old files keep their lookup tests and header. Only the page tests listed above leave
them (and the page half of `:199`), in the same PR that adds **V**, **S**, **E** and **T**.

### A.9 Data migration

None. No table, column or stored value changes. The views read what `DocumentsLookup` reads today.

---

## §B. The staff report

This section has its own file, `docs/design/xchange-v3-wave3-staff-report.md` (PR 533), so this PR did not change
under review.
