# Blueprint: users set the business values themselves

**Owner's direction** (controller chat, 2026-11-02, about 22:00 ICT, relayed by the lane controller claude/608dff):
- *"แนวทางการพัฒนาให้ ยึดหลัก ยูสเชอร์ ตั้งค่า ได้เอง"* (the development approach holds to one principle: users set the values themselves);
- *"ห้ามฟิก ใน โค้ก"* (never fix the values in code);
- *"ให้ owner ตั้งค่าพวกนี้ได้เอง"* (the Owner sets these values themselves);
- *"เจ้าของเป็น คนตั้งค่า ไว้ ให้ ฝ่ายการเงินปฏิบัติ ตาม"* (the owner sets the values; finance follows them), about 22:20 ICT.

## The rule

1. **Every business value is stored data, saved from a screen.** This covers limits, thresholds, levels, amounts, counts, durations a shop decides, and switches between business behaviours. Examples are the transfer approval limit, the count-difference limit, the drawer low-cash level and the alert floors. None of them is a constant in Rust or TypeScript, a config file, an environment variable or a migration's seed value.
2. **The Owner sets them; the other roles follow them.** Saving a business setting needs an EXECUTIVE-only permission, which the Owner account (ADMIN + EXECUTIVE) holds. Finance, branch managers and tellers may read a setting and must work within it, but never save it. In particular, finance does not set the limits that govern its own transfers and counts.
3. **"Not set" behaves safely, and is visible.** A missing value means the safe default the design names: no auto-approval, no warning, or an executive decision. It is never a hidden number. The screen shows that the value is not set.
4. **Changes take effect without a deploy.** They are versioned or stamped with who saved them and when, as the existing settings are (`updatedBy` / `setBy`).

**What is not a business value.** Protocol and safety bounds that no shop decides stay in code. Examples are field length bounds, retry limits, idempotency windows, the 15-second desk wait, and the sign-up throttle. If unsure, ask the lane controller; don't decide alone.

## How it applies

| Who | Must |
|---|---|
| Design author | Name where each business value is stored, which screen saves it, and which permission guards it. State that the permission is EXECUTIVE-only, so the Owner holds it and finance does not. |
| Ticket author | Never write a business number into code scope or tests as a constant. Tests that need a value set it through the save command or a fixture row. |
| Reviewer | A business value fixed in code, config or a seed is a **FAIL**, however small. |

## Already stored settings (measured on main, 2026-11-02)

| Value | Stored in | Saved by | Screen |
|---|---|---|---|
| Cash transfer approval limit, per currency | `cash_transfer_policy_versions` | `transferPolicySave` (`CashTransferPolicySet`; today module Finance/Full, so FINANCE also holds it, which must move to EXECUTIVE-only) | `web/src/screens/admin/cash-policy/CashPolicy.tsx` |
| Count-difference limit | `cash_limits` | `cashLimitsSave` (`CashLimitsManage`; today module Finance/Full, which must move to EXECUTIVE-only) | `#cash-limits` |
| Drawer low-cash level ([ledger-inventory](ledger-inventory.md) §5.1.3, being built) | `cash_drawer_low_levels` | `drawerLowLevelsSave` (inherits `cashLimitsSave`) | `#cash-limits` |
| Branch alert floors (EX-5) | `branch_stock_floors` | `branchAlertSettingsSave` (`AlertSettingsSave`: EXECUTIVE, ADMIN) | executive alert settings |
