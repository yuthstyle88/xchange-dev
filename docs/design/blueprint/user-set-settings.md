# Blueprint: users set the business values themselves

**Owner's direction** (controller chat, 2026-11-02, about 22:00 ICT, relayed by the lane controller claude/608dff):
- *"แนวทางการพัฒนาให้ ยึดหลัก ยูสเชอร์ ตั้งค่า ได้เอง"* (the development approach holds to one principle: users set the values themselves);
- *"ห้ามฟิก ใน โค้ก"* (never fix the values in code);
- *"ให้ owner ตั้งค่าพวกนี้ได้เอง"* (the Owner sets these values themselves);
- *"เจ้าของเป็น คนตั้งค่า ไว้ ให้ ฝ่ายการเงินปฏิบัติ ตาม"* (the owner sets the values; finance follows them), about 22:20 ICT;
- *"ทำเหมือน ระบบ ธนาคาร ทั่วไป"* (do it the way an ordinary banking system does);
- *"ตั้งค่าต้องมี otp หรือ ถ้า ฝ่ายตั้งได้ ต้องได้ การอนุมัติ"* (a settings change needs an OTP; if a department may set it, it needs approval);
- *"มันเป็นคำร้อง ให้ เจ้าของ อนยุมัติ"* (it is a request for the owner to approve);
- *"ให้ OTP ทาง อีเมล์"* (send the OTP by email);
- *"มี คอนฟิก ปรับเปลี่ยนได้"* (with a setting that can change it);
- *"เจ้าของเปลี่ยน เบอร์ OTPได้"* (the owner can change where the OTP goes).

## The rule

1. **Every business value is stored data, saved from a screen.** This covers limits, thresholds, levels, amounts, counts, durations a shop decides, and switches between business behaviours. Examples are the transfer approval limit, the count-difference limit, the drawer low-cash level and the alert floors. None of them is a constant in Rust or TypeScript, a config file, an environment variable or a migration's seed value.
2. **The Owner sets them; the other roles follow them.** Saving a business setting needs an EXECUTIVE-only permission, which the Owner account (ADMIN + EXECUTIVE) holds. Finance, branch managers and tellers may read a setting and must work within it, but never save it. In particular, finance does not set the limits that govern its own transfers and counts.
   - **The Owner confirms every save with an OTP** (step-up), sent **by email** to the Owner's registered email address. A save without a valid OTP changes nothing.
   - **The OTP channel is itself a setting.** Email is the default. The Owner can change it on a screen, for example to SMS once SMS delivery works. Changing the channel needs an OTP through the *current* channel, so a stolen session cannot redirect the codes. A channel that cannot deliver can't be chosen.
   - **The Owner can change the OTP destination**, meaning the email address or the phone number. As banks do, the change needs two codes: one through the current destination, to prove it is the Owner, and one sent to the new destination, to prove it reaches them. The change is recorded like any setting. A notice also goes to the old destination, so a change the Owner did not make is seen.
   - **A department may be delegated to request a setting change**, for example finance asking for a new count-difference limit. The change is filed as a **request for the Owner to approve**. It is an approval request in the existing approvals system (`approval_requests`, a new settings-change kind; ♻️ no second approval flow), carrying the old value, the new value and the reason. It appears on the Owner's approvals screen. It takes effect only when the Owner approves it, with the Owner's OTP. Until then, the current value stays in force. A rejected request changes nothing and keeps the Owner's reason. The requester can never approve their own request.
3. **"Not set" behaves safely, and is visible.** A missing value means the safe default the design names: no auto-approval, no warning, or an executive decision. It is never a hidden number. The screen shows that the value is not set.
4. **Changes take effect without a deploy.** They are versioned or stamped with who saved them and when, as the existing settings are (`updatedBy` / `setBy`).

## The banking model these rules follow

The owner asked for settings to work the way an ordinary banking system does. These five controls are the standard ones, and every settings design follows them.

1. **Segregation of duties.** The person who sets a limit is never the person the limit governs. The Owner sets policy. Finance, branch managers and tellers run operations within it. Nobody can raise the limit on their own work.
2. **A delegation-of-authority matrix.** The Owner decides who may approve what, and up to how much, per role and per currency. The cash transfer limit is the first row of that matrix. Approval levels are data in this matrix, never role checks written in code.
3. **Maker-checker on money operations.** A request is made by one person and approved by another. The approver is never the maker: a transfer, an adjustment, a reversal or a count difference. This is already the rule for approvals (§11.9); the matrix only says who the checker is.
4. **Settings changes are dual-controlled.** The Owner's OTP is required. A delegated department files a request, and the Owner approves it.
5. **A full audit trail of settings.** Every change of a setting keeps who changed it, when, the old value and the new value. Changes are kept as versions, not overwritten, and they can be read back on the screen.

**What is not a business value.** Protocol and safety bounds that no shop decides stay in code. Examples are field length bounds, retry limits, idempotency windows, the 15-second desk wait, and the sign-up throttle. If unsure, ask the lane controller; don't decide alone.

## How it applies

| Who | Must |
|---|---|
| Design author | Name where each business value is stored, which screen saves it, and which permission guards it. State that the permission is EXECUTIVE-only, so the Owner holds it and finance does not. |
| Ticket author | Never write a business number into code scope or tests as a constant. Tests that need a value set it through the save command or a fixture row. |
| Reviewer | A business value fixed in code, config or a seed is a **FAIL**, however small. |

**Designs that apply this rule:** [owner-set-values](owner-set-values.md) (the Owner's pricing rules, Group A, and the hidden fallbacks, Group D) and [pricing-tiers](pricing-tiers.md) §15.14 (tier bands, row 1).

## Every setting names who saved it, by name, from its own row (F4, F6) `[required]`

> designed-by: claude/b091f5e0 (BE sub-controller under claude/608dff), 2026-11-03, measured on `main` `eb2849a5`.
> Triage: #564 issuecomment-6596339013, F4 and F6 (order item B1, 2026-11-02). Versions and old values come later, with the shared settings-audit design (banking control 5). This section only makes today's "set by" true and readable.

**Measured.**
- **F6.** `cash_transfer_policy` answers `setBy` as the raw user id (`CashTransferPolicyData.set_by`, `api/src/presentation/dto/v3/cash_transfer_policy.rs:29`; view `views/cash_transfer_policy.rs:97`; save `commands/cash.rs:576-595`). `CashPolicy.tsx:213` prints it.
- **The same gap in `margin_guard`.** `MarginGuardData.set_by` is the raw id too (`dto/v3/margin_guard.rs:13`; view `views/margin_guard.rs:53-65`; save `commands/margin_guard_set.rs:95-99`).
- **F4.** `cash_limits` has one setter, the count-difference limit's: `updatedBy`, `updatedByName` and `updatedAt` (`dto/v3/cash_limits.rs:27-29`).
  - The drawer low levels carry none (`DrawerLowLevel {currency, lowLevel}`, `:12-14`), although `cash_drawer_low_levels` stores `updated_by` and `updated_at` per row.
  - `drawerLowLevelsSave` answers `CashLimitsData`, so after saving the levels the page names whoever last saved the *limit*. On eb2849a5 that was Finance.
- **The name join is copied.** `NULLIF(btrim(u.name), '')` with `LEFT JOIN users u ON u.id::text = …` appears in both `views/cash_limits.rs:27-31` and `commands/cash_limits_save.rs:74-76`.

**The rule.**
1. **One owner of "who is this user id".** `SetterNames::of_in(tx, ids) -> HashMap<String, String>` in `api/src/presentation/http/v3/setter_names.rs` (new). It runs one query over a set of ids with the existing expression (`NULLIF(btrim(name), '')`). A missing or blank name is absent from the map, so the field is `null`, never the id.
2. **Each setting's answer carries its own setter.**
   - `CashTransferPolicyData` gains `setByName: string | null`. `MarginGuardData` gains `setByName: string | null`.
   - `CashLimitsData` gains `drawerLowLevelsUpdatedBy`, `drawerLowLevelsUpdatedByName` and `drawerLowLevelsUpdatedAt`, all `string | null`. `drawerLowLevelsSave` replaces the set in one write (`DrawerLowLevels::replace_in`, `xc-cash/src/infrastructure/drawer_low_levels.rs:40`), so every row of a branch has the same setter, read from any one row. With no rows, all three are `null`.
   - The limit's own `updatedBy*` fields keep their meaning.
3. **`DrawerLowLevels` gains `set_meta_in(tx, branch) -> Option<(String, DateTime<Utc>)>`.** It stays the table's one reader (ledger-inventory §5.1.3).
4. **The existing cash_limits joins move onto `SetterNames`**, so the expression lives in one place.

| Class / function | File | Change |
|---|---|---|
| `SetterNames::of_in` | `api/src/presentation/http/v3/setter_names.rs` | new |
| `CashTransferPolicyData` | `api/src/presentation/dto/v3/cash_transfer_policy.rs` | `+set_by_name` |
| `cash_transfer_policy` view, `transferPolicySave` | `api/src/presentation/http/v3/views/cash_transfer_policy.rs`, `api/src/presentation/http/v3/commands/cash.rs` | fill it |
| `MarginGuardData` | `api/src/presentation/dto/v3/margin_guard.rs` | `+set_by_name` |
| `margin_guard` view, `marginGuardSet` | `api/src/presentation/http/v3/views/margin_guard.rs`, `api/src/presentation/http/v3/commands/margin_guard_set.rs` | fill it |
| `CashLimitsData` | `api/src/presentation/dto/v3/cash_limits.rs` | `+drawer_low_levels_updated_by`, `+…_by_name`, `+…_at` |
| `cash_limits` view, `cashLimitsSave`, `drawerLowLevelsSave` | `api/src/presentation/http/v3/views/cash_limits.rs`, `api/src/presentation/http/v3/commands/cash_limits_save.rs`, `api/src/presentation/http/v3/commands/drawer_low_levels_save.rs` | fill them; joins via `SetterNames` |
| `DrawerLowLevels::set_meta_in` | `api/crates/xc-cash/src/infrastructure/drawer_low_levels.rs` | new |
| generated TS and fixtures | `web/src/shared/api/generated/cash_transfer_policy.ts`, `web/src/shared/api/generated/margin_guard.ts`, `web/src/shared/api/generated/cash_limits.ts`, `web/src/shared/testing/fixtures/index.ts` | regenerated |

| # | Case | Required result |
|---|---|---|
| S-1 | The Owner (name "Owner A") saves the transfer policy; read `cash_transfer_policy` | `setBy` = the id, `setByName` = "Owner A" |
| S-2 | `transferPolicySave`'s own answer | the same `setByName` |
| S-3 | A setter whose `users.name` is blank | `setByName: null`, never the id |
| S-4 | `marginGuardSet`, then the `margin_guard` view | `setByName` = the setter's name |
| S-5 | Finance saves the count-difference limit, then the Owner saves the drawer levels | the `drawerLowLevelsSave` answer and the `cash_limits` view: `updatedByName` = Finance's name; `drawerLowLevelsUpdatedByName` = the Owner's name, with its own `…At` |
| S-6 | A branch with no drawer levels | the three `drawerLowLevels*` setter fields are `null` |
| S-7 | `cashLimitsSave` | `updatedByName` unchanged from today (pinned) |

**FE half (claude/01TxaRJG).**
- `#cash-policy` prints `setByName`, with "—" when it is `null`, never the id.
- The drawer low-level card on `#cash-limits` shows its own setter and time from the `drawerLowLevels*` fields.
- The page banner keeps naming the limit's setter.

**Data migration.** None: every column read already exists.

## Already stored settings (measured on main, 2026-11-02)

| Value | Stored in | Saved by | Screen |
|---|---|---|---|
| Cash transfer approval limit, per currency | `cash_transfer_policy_versions` | `transferPolicySave` (`CashTransferPolicySet`; today module Finance/Full, so FINANCE also holds it, which must move to EXECUTIVE-only) | `web/src/screens/admin/cash-policy/CashPolicy.tsx` |
| Count-difference limit | `cash_limits` | `cashLimitsSave` (`CashLimitsManage`; today module Finance/Full, which must move to EXECUTIVE-only) | `#cash-limits` |
| Drawer low-cash level ([ledger-inventory](ledger-inventory.md) §5.1.3, being built) | `cash_drawer_low_levels` | `drawerLowLevelsSave` (inherits `cashLimitsSave`) | `#cash-limits` |
| Branch alert floors (EX-5) | `branch_stock_floors` | `branchAlertSettingsSave` (`AlertSettingsSave`: EXECUTIVE, ADMIN) | executive alert settings |

A branch manager asks the Owner to raise a branch's OD (`SAFE_TO_DRAWER`) limit, temporarily or permanently, through the first settings-change approval kind: [od-limit-extension](od-limit-extension.md).
