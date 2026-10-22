# Design: admin campaigns Simulate sends a scenario (#4842)

> designed-by: claude/e8020026 (XC FE lane controller), 2026-10-23 ICT. Found by FE UI round 12 (claude/studio-claude-1,
> xchange #564 issuecomment-6353348357). Bug, not a feature: the blueprint's M02 row requires input (§1).

## 1. Bug and root cause

On admin `#campaigns`, **Simulate never works**. The server answers `400 validationFailed` on `scenarios` ("at least
one scenario is required"), so a draft can never reach `SIMULATED`, and `submit` refuses it with `simulationRequired`.

- `web/src/screens/admin/campaigns/Campaigns.tsx:520-531` `handleSimulate` always sends `{campaignId, scenarios: []}`.
- Since #1989 (`7358a299`, 2026-10-16), `campaign_simulate.rs:128-135` refuses an empty list. #1989 fixed the
  server's "an empty simulation counts as earned" bug (design D-4, *"`SIMULATED` is earned, not typed"*). The screen
  was never given the input.
- The blueprint already requires it: `docs/design/blueprint/marketing-campaigns.md:17`, the M02 route *"scenario cost,
  commission, benefits, contribution; input cost data ต้องระบุ"* (input cost data must be supplied).
- The server's scenario is `{branchId, currency, amount}`, all strings (`campaign_simulate.rs:41-44`). Every scenario
  in one call must target the same branch (`:248-254`). `amount` is positive, within the currency's minor units
  (`bounded-inputs.md:100`).

## 2. Rule

| # | Rule |
|---|---|
| SIM-1 | Next to the Simulate button, the detail form shows **one scenario row** (`<div class="xc-campaign-sim-row">`, each field in a `<label class="xc-campaign-sim-field">`) with three fields, `data-testid="sim-branch"`, `"sim-currency"` and `"sim-amount"`: **branch** (a `<select>`), **currency** (a 3-letter code input, upper-cased) and **amount** (a decimal text input). **Defaults:** currency `USD`, amount `100`. These keep the existing Simulate tests (`Campaigns.inflight.test.tsx` row 4 `:287-289`, `Campaigns.open-window.test.tsx:469`) and the e2e step (`flow-admin-campaigns.spec.ts:542-551`, MAIN has USD rates on the host) valid without changing them. |
| SIM-2 | **Branch options: the version's saved branches (`formBranches()`) only**, with the first preselected. The server refuses a scenario branch the version does not target (`campaign_simulate.rs` `branch_targeted`, ~`:217-238`). A version with no branches targets none, so it cannot be simulated (no `campaign_branches` rows; `simulator.rs:100`). When `formBranches()` is empty, the row shows **no select**, a worded line `<p class="xc-campaign-sim-note">` with the new key `simNeedsBranch` ("Add a branch to this version before simulating"), and Simulate stays disabled. |
| SIM-3 | **Simulate is disabled** (and `handleSimulate` returns before sending) until the branch is chosen, the currency is 3 letters A-Z, and the amount is a positive decimal. The existing `writing()` gate stays. |
| SIM-4 | `handleSimulate` sends `{campaignId, scenarios: [{branchId, currency, amount}]}`: exactly one scenario, with the amount as the typed string (trimmed). Nothing else in the body changes. |
| SIM-5 | A server refusal (e.g. `validationFailed` on `amount` or `currency`) shows through the existing `campaignError` path (`setDetailError`), as today; no new error handling. |
| SIM-6 | The scenario row keeps its values across `loadCampaignDetail` of the **same** campaign (after save or simulate), and resets to the defaults when a different campaign is selected or the page reloads. |

## 3. Ticket

| Ticket | File scope | Class | Base |
|---|---|---|---|
| #4842 | `web/src/screens/admin/campaigns/Campaigns.tsx` | logic | `feat/4842-sim-scenario`, created before release with: the i18n keys `simBranch`, `simCurrency`, `simAmount` and `simNeedsBranch` (th, en) in `campaigns/i18n.ts`; the CSS for `.xc-campaign-sim-row`, `.xc-campaign-sim-field` and `.xc-campaign-sim-note` in the screen's stylesheet (so the main-only `screen-look.test.ts` ratchet passes); and the up-front tests `Campaigns.simulate-scenario.test.tsx` |
