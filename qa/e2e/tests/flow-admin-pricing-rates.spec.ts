// QA-R9-6 — flow: pricing configure/approve, starting prices, rate-alert and branch alert settings (design docs/design/qa-e2e.md §9; ticket #4290).
// Written by ticket QA-R9-6 (#4290); this file is its whole File scope.
// Import only from './helpers' and './acting-branch'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Page } from '@playwright/test';
import { actingBranch, assertActingBranch, switchSessionBranch } from './acting-branch';
import {
  hasAccount,
  mePermissions,
  posted,
  sameOriginHeaders,
  shot,
  signIn,
  STAFF_LANGS,
  users,
  withLang,
  writeHeaders,
} from './helpers';
import * as startingPricesTable from '../../../web/src/screens/admin/starting-prices/i18n';

const RUN_MARKER = `qa-r9-6-${Date.now()}`;
const STAFF_TH_EN = STAFF_LANGS as readonly ('th' | 'en')[];

const ROUTE_PRICING_CONFIGURE = /^\/api\/pricing\/configure$/;
const ROUTE_PRICING_APPROVE = /^\/api\/pricing\/approve$/;
const ROUTE_PRICING_BRANCH_POLICY = /^\/api\/pricing\/branch-policy$/;
const ROUTE_RATE_POLICY_SAVE = /^\/api\/rate-policies$/;
const ROUTE_ORG_STARTING_PRICE_SAVE = /^\/api\/v3\/commands\/org_starting_price_save$/;
const ROUTE_RATE_ALERT_SETTINGS = /^\/api\/rate-alert-settings$/;
const ROUTE_BRANCH_ALERT_SETTINGS_SAVE = /^\/api\/v3\/commands\/branch_alert_settings_save$/;

// Screen i18n tables and exact refusal strings on origin/main:
// - web/src/screens/admin/pricing/i18n.ts:21,25,26,30,31,32,35 (th), :76,80,81,85,86,87,90 (en)
const PRICING_I18N = {
  th: {
    approve: 'อนุมัติ',
    cancel: 'ยกเลิก',
    noSuggestion: 'ยังไม่มีราคาแนะนำ',
    overridden: 'ตั้งเฉพาะสกุล',
    minSpread: 'ส่วนต่างขั้นต่ำ',
    save: 'บันทึก',
    reason: 'เหตุผล',
  },
  en: {
    approve: 'Approve',
    cancel: 'Cancel',
    noSuggestion: 'No suggestion yet',
    overridden: 'Currency override',
    minSpread: 'Minimum spread',
    save: 'Save',
    reason: 'Reason',
  },
} as const;

// PricingPolicy.tsx:99 renders wordedError(err), which returns errorText('validationFailed', err.message) = err.message
// (web/src/shared/i18n/worded-error.ts:16, index.ts:102-118; api/src/presentation/http/v3/commands/pricing_configure.rs:104,143;
// api/crates/xc-rate/src/domain/policy/currency_tier_override.rs:65; api/crates/xc-kernel/src/money/money.rs:16,19).
const PRICING_CONFIGURE_REFUSALS = {
  emptyReason: 'reason cannot be empty',
  negativeSpread: 'min_spread_pct must lie in 0..=5, got -0.5000',
  highSpread: 'min_spread_pct must lie in 0..=5, got 6.5000',
  tooManyDecimals: 'invalid buy margin in tier 1: too many decimal places',
  invalidMargin: 'invalid buy margin in tier 1: not a valid amount or rate',
} as const;

// api/src/presentation/http/v3/commands/pricing_approve.rs:125-215 refusal messages on 400 validationFailed.
const PRICING_APPROVE_REFUSALS = {
  emptyReason: 'reason must not be empty',
  emptyRows: 'rows must not be empty',
  wrongTierCount: 'each row must have 4 tiers (1..=4)',
  invalidBuyRate: 'invalid tier buy rate',
} as const;

// - web/src/screens/admin/starting-prices/i18n.ts:34-44 (th), :78-88 (en)
const STARTING_PRICES_I18N = {
  th: {
    reasonRequired: startingPricesTable.th['validation.reasonRequired'],
    positiveDecimal: startingPricesTable.th['validation.positiveDecimal'],
    buyAboveSell: startingPricesTable.th['validation.buyGreaterThanSell'],
    validationFailed: startingPricesTable.th['error.validationFailed'],
  },
  en: {
    reasonRequired: startingPricesTable.en['validation.reasonRequired'],
    positiveDecimal: startingPricesTable.en['validation.positiveDecimal'],
    buyAboveSell: startingPricesTable.en['validation.buyGreaterThanSell'],
    validationFailed: startingPricesTable.en['error.validationFailed'],
  },
} as const;

// - web/src/screens/admin/rate-alert-settings/i18n.ts:34-37 (th), :74-77 (en)
const RATE_ALERT_SETTINGS_I18N = {
  th: {
    errorCooldown: 'ระยะเวลา Cooldown ต้องเป็นตัวเลขบวกที่มากกว่า 0',
    errorMaxActive: 'จำนวนการแจ้งเตือนสูงสุดต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0',
    errorLifetime: 'อายุการแจ้งเตือนต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0 หรือเว้นว่างไว้',
  },
  en: {
    errorCooldown: 'Cooldown must be a positive number greater than 0',
    errorMaxActive: 'Max active alerts must be a positive integer greater than 0',
    errorLifetime: 'Lifetime must be a positive integer greater than 0 or left blank',
  },
} as const;

// - web/src/screens/admin/alert-settings/i18n.ts:2-6,14 (th), :19-23,31 (en)
const ALERT_SETTINGS_I18N = {
  th: {
    staleDataMinutes: 'ระยะเวลาข้อมูลค้าง (นาที)',
    save: 'บันทึก',
    addCurrency: 'เพิ่มสกุลเงิน',
    addFloor: 'เพิ่มเกณฑ์',
    removeFloor: 'ลบ',
    floorsHeading: 'เกณฑ์สต็อกขั้นต่ำรายสกุลเงิน',
  },
  en: {
    staleDataMinutes: 'Stale Data (minutes)',
    save: 'Save',
    addCurrency: 'Add Currency',
    addFloor: 'Add Floor',
    removeFloor: 'Remove',
    floorsHeading: 'Currency Stock Floors',
  },
} as const;

// AlertSettings.tsx:136 renders wordedError(err), which returns err.message for 400 validationFailed
// (api/src/presentation/http/v3/commands/branch_alert_settings_save.rs:64,79-83,100; org_view.rs:98).
const BRANCH_ALERT_REFUSALS = {
  staleMinutesNonPositive: 'staleDataMinutes must be greater than 0',
  invalidFloorDecimal: 'ข้อมูลไม่ถูกต้อง',
  unknownCurrency: (code: string) => `currency ${code} not in currencies`,
} as const;

// Field error for #stale-data-minutes (AlertSettings.tsx:158; alert-settings/i18n.ts:16,36).
const STALE_MINUTES_ERR = {
  th: 'กรุณาระบุจำนวนนาทีเป็นจำนวนเต็มบวกไม่เกิน 2,147,483,647',
  en: 'Enter a positive integer up to 2,147,483,647',
} as const;

// Field error for minQuantity in floor-row (AlertSettings.tsx:168; alert-settings/i18n.ts:17,37).
const FLOOR_QUANTITY_ERR = {
  th: 'กรุณาระบุสต็อกขั้นต่ำไม่ติดลบ ทศนิยมไม่เกิน 4 ตำแหน่ง',
  en: 'Enter a non-negative number with up to 4 decimal places',
} as const;

/** Waits for one PUT whose path matches while `act` runs; mirrors `posted` (`helpers.ts:365-371`) for PUT commands. */
async function putted(
  page: Page,
  path: RegExp,
  act: () => Promise<void>,
): Promise<{ ok: boolean; text: string; body: string }> {
  const res = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && path.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  await act();
  const r = await res;
  const body = await r.text().catch(() => '');
  return { ok: r.ok(), text: `${r.status()} ${body.slice(0, 300)}`, body };
}

interface PricingTierMarginDto {
  tierNo: number;
  buyMarginPct: string;
  sellMarginPct: string;
}

interface PricingTierRateDto {
  tierNo: number;
  buy: string | null;
  sell: string | null;
}

interface PricingBranchPolicyDto {
  buyBp: number;
  sellBp: number;
  own: boolean;
  follow: boolean;
  from: string;
}

interface PricingRowDto {
  currency: string;
  published: PricingTierRateDto[];
  margins: PricingTierMarginDto[];
  overridden: boolean;
  minSpreadPct: string | null;
  suggestion: PricingTierRateDto[] | null;
  selectable: boolean;
  tierSet: { id: string } | null;
  branchPolicy: PricingBranchPolicyDto;
}

interface PricingBoardDto {
  branch: string;
  policyVersionId: string | null;
  valueTiersOn: boolean;
  rows: PricingRowDto[];
}

function hasFourCompleteTiers(
  tiers: PricingTierRateDto[] | null | undefined,
): tiers is Array<{ tierNo: number; buy: string; sell: string }> {
  return (
    Array.isArray(tiers) &&
    tiers.length === 4 &&
    tiers.every((t) => typeof t.buy === 'string' && t.buy !== '' && typeof t.sell === 'string' && t.sell !== '')
  );
}

// Reads GET /api/v3/views/pricing_board?branch=<branch> (web/src/screens/admin/pricing/Pricing.tsx:66-70).
async function fetchPricingBoard(page: Page, branch = users.branch): Promise<PricingBoardDto | null> {
  const res = await page.request.get(`/api/v3/views/pricing_board?branch=${encodeURIComponent(branch)}`);
  if (!res.ok()) return null;
  const body = (await res.json().catch(() => null)) as { data?: PricingBoardDto } | null;
  return body?.data ?? null;
}

// Opens a currency row on /#pricing (Pricing.tsx:153,209; PricingRow.tsx:36,49).
async function openPricingRow(page: Page, currency: string): Promise<void> {
  const screen = page.locator('.pricing-screen');
  await expect(screen).toBeVisible({ timeout: 20_000 });
  const rowBtn = screen.locator(`tr.pricing-row[data-currency="${currency}"] .pricing-btn-code`).first();
  await expect(rowBtn, `pricing row button for ${currency}`).toBeVisible({ timeout: 20_000 });
  await rowBtn.click();
  await expect(screen.locator('.pricing-detail')).toBeVisible({ timeout: 20_000 });
}

async function ensureBranch2Priced(
  page: Page,
  branch2: string,
  role: 'executive' | 'admin',
): Promise<string | null> {
  try {
    let board = await fetchPricingBoard(page, branch2);
    if (board?.policyVersionId && board.rows.some((r) => hasFourCompleteTiers(r.published))) {
      return null;
    }

    if (!board?.policyVersionId) {
      if (role !== 'executive') {
        return 'branch2 has no rate policy and only executive holds RatePolicySave (role_grants.rs: ADMIN has no RatesStock)';
      }

      const mainPolicyRes = await page.request.get(
        `/api/v3/views/rate_policy?branch=${encodeURIComponent(users.branch)}`,
      );
      let maxReferenceAgeHours = 24;
      if (mainPolicyRes.ok()) {
        const body = (await mainPolicyRes.json().catch(() => null)) as {
          data?: { policy?: { maxReferenceAgeHours?: number | null } | null };
        } | null;
        if (body?.data?.policy?.maxReferenceAgeHours != null) {
          maxReferenceAgeHours = body.data.policy.maxReferenceAgeHours;
        }
      }

      await page.goto('/#rates');
      const screen = page.locator('.admin-rates-screen');
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const policyCard = screen.locator('.rates-section').filter({ has: page.getByTestId('save-policy') });
      await expect(policyCard).toBeVisible({ timeout: 20_000 });
      const ageInput = policyCard.locator('input[name="maxReferenceAgeHours"]');
      await expect(ageInput).toBeVisible({ timeout: 20_000 });
      await ageInput.fill(String(maxReferenceAgeHours));
      // Save stays disabled for a branch with no policy until #5785 is deployed (RatesPolicyCard.tsx:518-524). Never click a
      // disabled button through posted(): its waitForResponse (helpers.ts:431) would reject unawaited and fail the test.
      const saveEnabled = await expect(policyCard.getByTestId('save-policy'))
        .toBeEnabled({ timeout: 20_000 })
        .then(() => true, () => false);
      if (!saveEnabled) {
        return `first policy on ${branch2} failed: save-policy stays disabled (is #5785 deployed on this host?)`;
      }

      try {
        await assertActingBranch(page, branch2, 'setup: first policy');
        const res = await posted(page, ROUTE_RATE_POLICY_SAVE, () =>
          policyCard.getByTestId('save-policy').click(),
        );
        if (!res.ok) {
          return `first policy on ${branch2} failed: ${res.text} ${res.body}`;
        }
      } catch (err: unknown) {
        return `first policy on ${branch2} failed: ${err instanceof Error ? err.message : String(err)}`;
      }

      console.log(`[qa-r9-6] setup: first rate policy on ${branch2}`);
    }

    board = await fetchPricingBoard(page, branch2);
    if (!board?.rows.some((r) => hasFourCompleteTiers(r.published))) {
      const candidates = (board?.rows ?? []).filter(
        (r) => r.selectable === true && hasFourCompleteTiers(r.suggestion),
      );
      const row = candidates.find((r) => r.currency === 'USD') ?? candidates[0];
      if (!row) {
        return `no row on branch ${branch2} has selectable=true and 4 complete suggestion tiers`;
      }

      await page.goto('/#pricing');
      await openPricingRow(page, row.currency);

      const tierEditor = page.locator('section.tier-editor');
      await expect(tierEditor).toBeVisible({ timeout: 20_000 });
      await tierEditor.locator('input[name="reason"]').fill(`${RUN_MARKER}-setup-first-publication`);
      const approveEnabled = await expect(tierEditor.locator('button[name="approve"]'))
        .toBeEnabled({ timeout: 20_000 })
        .then(() => true, () => false);
      if (!approveEnabled) {
        return `first publication on ${branch2} failed: approve stays disabled for ${row.currency}`;
      }

      let approveRes: { ok: boolean; text: string; body: string };
      try {
        await assertActingBranch(page, branch2, 'setup: first publication');
        approveRes = await posted(page, ROUTE_PRICING_APPROVE, () =>
          tierEditor.locator('button[name="approve"]').click(),
        );
      } catch (err: unknown) {
        return `first publication on ${branch2} failed: ${err instanceof Error ? err.message : String(err)}`;
      }
      if (!approveRes.ok) {
        return approveRes.body || approveRes.text;
      }

      let approveBody: { currencies?: Array<{ currency?: string; state?: string }> } | null = null;
      try {
        approveBody = JSON.parse(approveRes.body) as {
          currencies?: Array<{ currency?: string; state?: string }>;
        };
      } catch {
        return approveRes.body;
      }
      if (approveBody?.currencies?.[0]?.state !== 'PUBLISHED') {
        return approveRes.body;
      }

      console.log(
        `[qa-r9-6] setup: first publication of ${row.currency} on ${branch2} (one-time, not restored: the branch had none; #564 ruling)`,
      );
    }

    return null;
  } catch (err: unknown) {
    return `ensureBranch2Priced failed: ${err instanceof Error ? err.message : String(err)}`;
  }
}

test.describe.serial('QA-R9-6 flow: admin pricing configure/approve, starting prices, rate-alert and branch alert settings', () => {
  // ── (a) Happy path ────────────────────────────────────────────────────────

  test('orgStartingPriceSave (POST /api/v3/commands/org_starting_price_save): saves starting prices on /#starting-prices, reads back after reload, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/starting-prices/StartingPrices.tsx:322,382,427,442,468,482,494,509 (menu.ts:13 needs OrgStartingPricesManage)
    await signIn(page, 'executive', '/#starting-prices');
    const screen = page.locator('.xc-sp-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const usdRow = screen.locator('tr.xc-sp-currency-row[data-currency="USD"]');
    if ((await usdRow.count()) === 0) {
      await screen.locator('input.xc-sp-add-code-input').fill('USD');
      await screen.locator('button.xc-sp-add-code-btn').click();
    }
    await expect(usdRow).toBeVisible({ timeout: 20_000 });

    const buyInput = usdRow.locator('input.xc-sp-input[data-field="buy"]');
    const sellInput = usdRow.locator('input.xc-sp-input[data-field="sell"]');
    const reasonInput = screen.locator('input#xc-sp-reason[name="reason"]');
    const saveBtn = screen.locator('button.xc-sp-save-btn[data-action="save"]');

    const origBuyRaw = (await buyInput.inputValue()).trim();
    const origSellRaw = (await sellInput.inputValue()).trim();
    const origBuy = Number(origBuyRaw) > 0 ? origBuyRaw : '32.10';
    const origSell = Number(origSellRaw) > Number(origBuy) ? origSellRaw : '32.60';

    const nextBuy = Math.abs(Number(origBuy) - 32.15) < 0.001 ? '32.25' : '32.15';
    const nextSell = Math.abs(Number(origSell) - 32.75) < 0.001 ? '32.85' : '32.75';
    const reasonText = `${RUN_MARKER}-starting-prices`;

    let restored = false;
    const restoreStartingPrices = async () => {
      if (restored) return;
      await page.goto('/#starting-prices');
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await buyInput.fill(origBuy);
      await sellInput.fill(origSell);
      await reasonInput.fill(`${RUN_MARKER}-restore-sp`);
      const res = await posted(page, ROUTE_ORG_STARTING_PRICE_SAVE, () => saveBtn.click());
      expect(res.ok, `restore org_starting_price_save: ${res.text}`).toBeTruthy();
      restored = true;
    };

    try {
      await buyInput.fill(nextBuy);
      await sellInput.fill(nextSell);
      await reasonInput.fill(reasonText);

      const saved = await posted(page, ROUTE_ORG_STARTING_PRICE_SAVE, () => saveBtn.click());
      expect(saved.ok, `POST /api/v3/commands/org_starting_price_save: ${saved.text}`).toBeTruthy();
      await shot(page, 'flow-admin-pricing-rates', '1-starting-prices-saved');

      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await expect(screen.locator('.xc-sp-history-meta')).toContainText(reasonText, { timeout: 20_000 });
      expect(Number(await buyInput.inputValue())).toBeCloseTo(Number(nextBuy), 4);
      expect(Number(await sellInput.inputValue())).toBeCloseTo(Number(nextSell), 4);

      await restoreStartingPrices();
    } finally {
      await restoreStartingPrices().catch((e) => console.warn('[starting-prices] restore failed:', e));
    }
  });

  test('pricingConfigure (POST /api/pricing/configure): admin configures currency override, finance reads back on /#pricing, reload read-back, and restores before test ends and in finally', async ({
    browser,
  }) => {
    // web/src/screens/admin/pricing/Pricing.tsx:153,209; PricingRow.tsx:36,49,51; PricingPolicy.tsx:129,168,182,201,216,228,236
    const adminPage = await browser.newPage({ baseURL: users.baseUrl });
    const financePage = hasAccount('finance') ? await browser.newPage({ baseURL: users.baseUrl }) : null;

    try {
      await signIn(adminPage, 'executive', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
      const initialBoard = await fetchPricingBoard(adminPage, users.branch);
      if (!initialBoard?.policyVersionId || !initialBoard.valueTiersOn || initialBoard.rows.length === 0) {
        const reason = `pricing_board on branch ${users.branch} is not ready (policyVersionId=${initialBoard?.policyVersionId ?? 'null'}, valueTiersOn=${String(initialBoard?.valueTiersOn)}, rows=${initialBoard?.rows.length ?? 0})`;
        console.log(`[qa-r9-6] pricingConfigure: skipping — ${reason}`);
        test.skip(true, reason);
        return;
      }

      const targetRow = initialBoard.rows.find((r) => r.currency === 'USD') ?? initialBoard.rows[0]!;
      const currency = targetRow.currency;
      const wasOverridden = Boolean(targetRow.overridden);
      const origMinSpread = targetRow.minSpreadPct ?? '';

      await adminPage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
      await openPricingRow(adminPage, currency);

      const policyForm = adminPage.locator('form.pricing-policy-form');
      await expect(policyForm).toBeVisible({ timeout: 20_000 });

      const origBuyMargins: Record<number, string> = {};
      const origSellMargins: Record<number, string> = {};
      for (const t of [1, 2, 3, 4]) {
        origBuyMargins[t] = (await policyForm.locator(`input[name="buy-margin-${t}"]`).inputValue()).trim() || '0';
        origSellMargins[t] = (await policyForm.locator(`input[name="sell-margin-${t}"]`).inputValue()).trim() || '0';
      }

      let restored = false;
      const restorePricingPolicy = async () => {
        if (restored) return;
        await adminPage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
        await openPricingRow(adminPage, currency);
        if (!wasOverridden) {
          await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-restore-clear`);
          const clearBtn = policyForm.locator('button[name="clearOverride"]');
          const revCfg = await posted(adminPage, ROUTE_PRICING_CONFIGURE, () => clearBtn.click());
          expect(revCfg.ok, `restore clearOverride: ${revCfg.text}`).toBeTruthy();
        } else {
          for (const t of [1, 2, 3, 4]) {
            await policyForm.locator(`input[name="buy-margin-${t}"]`).fill(origBuyMargins[t]);
            await policyForm.locator(`input[name="sell-margin-${t}"]`).fill(origSellMargins[t]);
          }
          await policyForm.locator('input[name="minSpreadPct"]').fill(origMinSpread);
          await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-restore-policy`);
          const saveBtn = policyForm.locator('button[name="save"]');
          const revCfg = await posted(adminPage, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
          expect(revCfg.ok, `restore pricingConfigure: ${revCfg.text}`).toBeTruthy();
        }
        restored = true;
      };

      try {
        // Use small delta within existing margins so any currency guard is respected, and distinct minSpreadPct in 0..=5.
        const cfgBuy = [1, 2, 3, 4].map((t) => {
          const base = Number(origBuyMargins[t]) || 0;
          return (Math.abs(base - 0.05 * t) < 0.001 ? base + 0.01 : base).toFixed(4);
        });
        const cfgSell = [1, 2, 3, 4].map((t) => {
          const base = Number(origSellMargins[t]) || 0;
          return base.toFixed(4);
        });
        const cfgSpread = Math.abs(Number(origMinSpread || '0') - 0.25) < 0.001 ? '0.35' : '0.25';
        const cfgReason = `${RUN_MARKER}-configure`;

        for (const t of [1, 2, 3, 4]) {
          await policyForm.locator(`input[name="buy-margin-${t}"]`).fill(cfgBuy[t - 1]!);
          await policyForm.locator(`input[name="sell-margin-${t}"]`).fill(cfgSell[t - 1]!);
        }
        await policyForm.locator('input[name="minSpreadPct"]').fill(cfgSpread);
        await policyForm.locator('input[name="reason"]').fill(cfgReason);

        const cfgRes = await posted(adminPage, ROUTE_PRICING_CONFIGURE, () =>
          policyForm.locator('button[name="save"]').click(),
        );
        expect(cfgRes.ok, `POST /api/pricing/configure: ${cfgRes.text}`).toBeTruthy();
        await shot(adminPage, 'flow-admin-pricing-rates', '2-pricing-configured');

        // Reload and verify configured margins + overridden badge persist.
        await adminPage.reload();
        await openPricingRow(adminPage, currency);
        await expect(
          adminPage.locator(`tr.pricing-row[data-currency="${currency}"] .pricing-badge-overridden`),
          `${currency} row shows overridden badge after pricingConfigure (PricingRow.tsx:51)`,
        ).toBeVisible({ timeout: 20_000 });
        for (const t of [1, 2, 3, 4]) {
          expect(Number(await policyForm.locator(`input[name="buy-margin-${t}"]`).inputValue())).toBeCloseTo(
            Number(cfgBuy[t - 1]),
            4,
          );
          expect(Number(await policyForm.locator(`input[name="sell-margin-${t}"]`).inputValue())).toBeCloseTo(
            Number(cfgSell[t - 1]),
            4,
          );
        }
        expect(Number(await policyForm.locator('input[name="minSpreadPct"]').inputValue())).toBeCloseTo(
          Number(cfgSpread),
          4,
        );

        // Cross-check on finance's /#pricing view (smoke-admin-pricing.spec.ts:21-34: finance holds ReferenceRatesRead).
        if (financePage) {
          await signIn(financePage, 'finance', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
          await financePage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
          await openPricingRow(financePage, currency);
          await expect(
            financePage.locator(`tr.pricing-row[data-currency="${currency}"] .pricing-badge-overridden`),
          ).toBeVisible({ timeout: 20_000 });
        }

        // Restore pricing BEFORE the test ends, and verify after reload.
        await restorePricingPolicy();
        await adminPage.reload();
        await openPricingRow(adminPage, currency);
        if (!wasOverridden) {
          await expect(
            adminPage.locator(`tr.pricing-row[data-currency="${currency}"] .pricing-badge-overridden`),
          ).toHaveCount(0);
        }
      } finally {
        await restorePricingPolicy().catch((e) => console.warn('[pricingConfigure] restore failed:', e));
      }
    } finally {
      await financePage?.close().catch(() => {});
      await adminPage.close().catch(() => {});
    }
  });

  // Runs on users.branch2 (never MAIN) via session branch switch:
  // The admin shell reads only me.branch, never ?branch= (AdminShell.tsx:243-261), and switching writes
  // users.last_branch_id (session_service.rs:1090-1099) which persists across sign-ins. The test switches to branch2
  // through the account bar, asserts the acting branch on screen before every write, and always switches back in finally.
  // 1. Sets up TEST2's first rate policy and publication through the UI when missing (ensureBranch2Priced).
  // 2. Skips with a logged reason when branch2 is unset, pricing_board or policyVersionId is null, or no selectable
  //    row has 4 complete suggestion tiers and 4 complete published tiers (prefers USD).
  // 3. Records published (4 tiers) and branchPolicy (buyBp, sellBp, own, follow) before the approve.
  // 4. Approves the prefilled suggestion on /#pricing via TierEditor (POST /api/pricing/approve)
  //    and expects currencies[0].state === 'PUBLISHED' (pricing_approve.rs:482-497 sets own=true on non-MAIN branches;
  //    pricing_approve.rs:511-535 pushes to every other branch only on MAIN).
  // 5. Reads back pricing_board and checks published equals the approved suggestion (to 4 dp).
  // 6. Restores in a single-run restore() closure (called before test end and in finally):
  //    (i) POST /api/pricing/approve with branch: branch2 in the body and recorded published tiers (retrying once on staleVersion/conflict);
  //    (ii) if recorded own was false or follow/buyBp/sellBp differ, saves the branch-policy form back via UI (save-policy).
  // 7. Verifies restored published (to 4 dp) and branchPolicy (own, follow, buyBp, sellBp).
  test('pricingApprove (POST /api/pricing/approve): approves a suggested tier set on branch2, reads back published rates, and restores published rates and branch policy', async ({
    page,
  }) => {
    if (!users.branch2) {
      const reason = 'no branch2 in users.json: run scripts/reseed.sh';
      console.log(`[qa-r9-6] pricingApprove: skipping — ${reason}`);
      test.skip(true, reason);
      return;
    }

    const branch2 = users.branch2;
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#pricing');

    await switchSessionBranch(page, branch2);
    try {
      const setupReason = await ensureBranch2Priced(page, branch2, role);
      if (setupReason) console.log(`[qa-r9-6] pricingApprove: setup did not complete — ${setupReason}`);

      const initialBoard = await fetchPricingBoard(page, branch2);
      if (!initialBoard?.policyVersionId) {
        const reason = `pricing_board on branch ${branch2} is not ready (policyVersionId=${initialBoard?.policyVersionId ?? 'null'})`;
        console.log(`[qa-r9-6] pricingApprove: skipping — ${reason}`);
        test.skip(true, reason);
        return;
      }

      const eligibleRows = initialBoard.rows.filter(
        (r) => r.selectable === true && hasFourCompleteTiers(r.suggestion) && hasFourCompleteTiers(r.published),
      );
      const targetRow = eligibleRows.find((r) => r.currency === 'USD') ?? eligibleRows[0];
      if (!targetRow || !hasFourCompleteTiers(targetRow.suggestion) || !hasFourCompleteTiers(targetRow.published)) {
        const reason = `no row on branch ${branch2} has selectable=true, 4 complete suggestion tiers, and 4 complete published tiers`;
        console.log(`[qa-r9-6] pricingApprove: skipping — ${reason}`);
        test.skip(true, reason);
        return;
      }

      const currency = targetRow.currency;
      const recordedPublished = targetRow.published.map((t) => ({
        tierNo: t.tierNo,
        buy: t.buy,
        sell: t.sell,
      }));
      const recordedPolicy = {
        buyBp: targetRow.branchPolicy.buyBp,
        sellBp: targetRow.branchPolicy.sellBp,
        own: targetRow.branchPolicy.own,
        follow: targetRow.branchPolicy.follow,
      };
      const recordedStateText = `recorded published=${JSON.stringify(recordedPublished)}, branchPolicy=${JSON.stringify(recordedPolicy)}`;

      let publishedByTest = false;
      let restored = false;
      const restore = async () => {
        if (restored) return;

        if (publishedByTest) {
          const postRestoreApprove = async () => {
            const curBoard = await fetchPricingBoard(page, branch2);
            const curRow = curBoard?.rows.find((r) => r.currency === currency);
            if (!curBoard?.policyVersionId || !curRow) {
              return {
                ok: false,
                retryable: false,
                detail: `pricing_board unavailable on ${branch2} (policyVersionId=${curBoard?.policyVersionId ?? 'null'})`,
              };
            }
            await assertActingBranch(page, branch2, 'restore (i)');
            // The command reads `branch` only from the body (command.rs:336-365); URL query dropped.
            const res = await page.request.post('/api/pricing/approve', {
              headers: writeHeaders(crypto.randomUUID()),
              data: {
                branch: branch2,
                policyVersionId: curBoard.policyVersionId,
                rows: [
                  {
                    currency,
                    tiers: recordedPublished,
                    ...(curRow.tierSet?.id ? { tierSetId: curRow.tierSet.id } : {}),
                  },
                ],
                reason: `${RUN_MARKER}-restore`,
              },
            });
            const bodyText = await res.text().catch(() => '');
            type ApproveResponseBody = {
              error?: { code?: string };
              currencies?: Array<{ currency?: string; state?: string }>;
            };
            let parsed: ApproveResponseBody | null = null;
            try {
              parsed = JSON.parse(bodyText) as ApproveResponseBody;
            } catch {
              // non-JSON
            }
            const code = parsed?.error?.code ?? '';
            const retryable =
              res.status() === 409 ||
              code === 'staleVersion' ||
              code === 'conflict' ||
              /staleVersion|conflict/.test(bodyText);
            const ok = res.ok() && parsed?.currencies?.[0]?.state === 'PUBLISHED';
            return { ok, retryable, detail: `${res.status()} ${bodyText}` };
          };

          let attempt = await postRestoreApprove();
          if (!attempt.ok && attempt.retryable) {
            attempt = await postRestoreApprove();
          }
          expect(
            attempt.ok,
            `restore (i) POST /api/pricing/approve failed (${attempt.detail}); ${recordedStateText}`,
          ).toBeTruthy();
        }

        const boardAfterApproveRestore = await fetchPricingBoard(page, branch2);
        const rowAfterApproveRestore = boardAfterApproveRestore?.rows.find((r) => r.currency === currency);
        const curBp = rowAfterApproveRestore?.branchPolicy;
        const needsPolicyRestore =
          !recordedPolicy.own ||
          !curBp ||
          curBp.own !== recordedPolicy.own ||
          curBp.follow !== recordedPolicy.follow ||
          curBp.buyBp !== recordedPolicy.buyBp ||
          curBp.sellBp !== recordedPolicy.sellBp;

        if (needsPolicyRestore) {
          await page.goto('/#pricing');
          await openPricingRow(page, currency);
          const branchPolicyForm = page
            .locator('.pricing-detail form.pricing-policy-form')
            .filter({ has: page.locator('button[name="save-policy"]') });
          await expect(branchPolicyForm).toBeVisible({ timeout: 20_000 });
          await branchPolicyForm.locator('input[name="buyBp"]').fill(String(recordedPolicy.buyBp));
          await branchPolicyForm.locator('input[name="sellBp"]').fill(String(recordedPolicy.sellBp));
          await branchPolicyForm.locator('input[name="own"]').setChecked(recordedPolicy.own);
          await branchPolicyForm.locator('input[name="follow"]').setChecked(recordedPolicy.follow);
          await assertActingBranch(page, branch2, 'restore (ii): branch policy');
          const polRes = await posted(page, ROUTE_PRICING_BRANCH_POLICY, () =>
            branchPolicyForm.locator('button[name="save-policy"]').click(),
          );
          expect(
            polRes.ok,
            `restore (ii) POST /api/pricing/branch-policy failed: ${polRes.text} ${polRes.body}; ${recordedStateText}`,
          ).toBeTruthy();
        }

        restored = true;
      };

      try {
        await page.goto('/#pricing');
        await openPricingRow(page, currency);

        const tierEditor = page.locator('section.tier-editor');
        await expect(tierEditor).toBeVisible({ timeout: 20_000 });
        await tierEditor.locator('input[name="reason"]').fill(`${RUN_MARKER}-approve`);
        // What the TierEditor sends (TierEditor.tsx:63-66), read after render: the suggestion moves with live quotes.
        const approvedSuggestion: Array<{ tierNo: number; buy: string; sell: string }> = [];
        for (const tierNo of [1, 2, 3, 4]) {
          approvedSuggestion.push({
            tierNo,
            buy: (await tierEditor.locator(`input[name="buy-${tierNo}"]`).inputValue()).trim(),
            sell: (await tierEditor.locator(`input[name="sell-${tierNo}"]`).inputValue()).trim(),
          });
        }

        await assertActingBranch(page, branch2, 'pricingApprove: approve suggestion');
        const approveRes = await posted(page, ROUTE_PRICING_APPROVE, () =>
          tierEditor.locator('button[name="approve"]').click(),
        );
        expect(approveRes.ok, `POST /api/pricing/approve: ${approveRes.text} ${approveRes.body}`).toBeTruthy();
        const approveBody = JSON.parse(approveRes.body) as {
          currencies?: Array<{ currency?: string; state?: string }>;
        };
        expect(approveBody.currencies?.[0]?.state, `POST /api/pricing/approve body: ${approveRes.body}`).toBe(
          'PUBLISHED',
        );
        publishedByTest = true;

        const afterApproveBoard = await fetchPricingBoard(page, branch2);
        const afterApproveRow = afterApproveBoard?.rows.find((r) => r.currency === currency);
        expect(afterApproveRow, `row ${currency} after approve`).toBeTruthy();
        for (const s of approvedSuggestion) {
          const pubTier = afterApproveRow!.published.find((t) => t.tierNo === s.tierNo);
          expect(pubTier, `published tier ${s.tierNo} after approve`).toBeTruthy();
          expect(Number(pubTier!.buy)).toBeCloseTo(Number(s.buy), 4);
          expect(Number(pubTier!.sell)).toBeCloseTo(Number(s.sell), 4);
        }

        await restore();

        const restoredBoard = await fetchPricingBoard(page, branch2);
        const restoredRow = restoredBoard?.rows.find((r) => r.currency === currency);
        expect(restoredRow, `row ${currency} after restore`).toBeTruthy();
        for (const rec of recordedPublished) {
          const pubTier = restoredRow!.published.find((t) => t.tierNo === rec.tierNo);
          expect(pubTier, `restored published tier ${rec.tierNo}`).toBeTruthy();
          expect(Number(pubTier!.buy)).toBeCloseTo(Number(rec.buy), 4);
          expect(Number(pubTier!.sell)).toBeCloseTo(Number(rec.sell), 4);
        }
        expect(restoredRow!.branchPolicy.own).toBe(recordedPolicy.own);
        expect(restoredRow!.branchPolicy.follow).toBe(recordedPolicy.follow);
        expect(restoredRow!.branchPolicy.buyBp).toBe(recordedPolicy.buyBp);
        expect(restoredRow!.branchPolicy.sellBp).toBe(recordedPolicy.sellBp);
      } finally {
        // A restore failure here must not hide the error that sent us to finally; it is logged with the recorded state.
        await restore().catch((e) => console.warn(`[pricingApprove] restore failed: ${String(e)}; ${recordedStateText}`));
      }
    } finally {
      await switchSessionBranch(page, users.branch);
      expect(
        (await actingBranch(page)).branch,
        'executive left on users.branch (last_branch_id persists to the next sign-in)',
      ).toBe(users.branch);
    }
  });

  test('rateAlertSettingsSave (PUT /api/rate-alert-settings): saves settings on /#rate-alert-settings, reads back after reload, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:211,242,258,289,320,353,433 (menu.ts:13 needs RateAlertSettingsSave)
    await signIn(page, 'executive', '/#rate-alert-settings');
    const screen = page.locator('.rate-alert-settings');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const cooldownInput = screen.locator('input#default-cooldown-hours[name="defaultCooldownHours"]');
    const maxActiveInput = screen.locator('input#max-active-per-customer[name="maxActivePerCustomer"]');
    const lifetimeInput = screen.locator('input#default-lifetime-days[name="defaultLifetimeDays"]');
    const saveBtn = screen.locator('button.rate-alert-submit-btn[type="submit"]');

    await expect(cooldownInput).toBeVisible({ timeout: 20_000 });

    const origCooldownRaw = (await cooldownInput.inputValue()).trim();
    const origMaxActiveRaw = (await maxActiveInput.inputValue()).trim();
    const origLifetimeRaw = (await lifetimeInput.inputValue()).trim();
    const origCooldown = Number(origCooldownRaw) > 0 ? origCooldownRaw : '24';
    const origMaxActive =
      Number.isInteger(Number(origMaxActiveRaw)) && Number(origMaxActiveRaw) > 0 ? origMaxActiveRaw : '10';
    const origLifetime =
      origLifetimeRaw === '' || (Number.isInteger(Number(origLifetimeRaw)) && Number(origLifetimeRaw) > 0)
        ? origLifetimeRaw
        : '30';

    let origPolicy: 'ALL_FRESH' | 'ALL' | 'AUTO_FRESH' = 'ALL_FRESH';
    for (const candidate of ['ALL_FRESH', 'ALL', 'AUTO_FRESH'] as const) {
      const radio = screen.locator(`input[type="radio"][name="observePolicy"][value="${candidate}"]`);
      if (await radio.isChecked().catch(() => false)) {
        origPolicy = candidate;
        break;
      }
    }

    const nextCooldown = origCooldown === '12' ? '24' : '12';
    const nextMaxActive = origMaxActive === '15' ? '10' : '15';
    const nextLifetime = origLifetime === '45' ? '30' : '45';
    const nextPolicy = origPolicy === 'ALL_FRESH' ? 'ALL' : 'ALL_FRESH';

    let restored = false;
    const restoreRateAlertSettings = async () => {
      if (restored) return;
      await page.goto('/#rate-alert-settings');
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await cooldownInput.fill(origCooldown);
      await maxActiveInput.fill(origMaxActive);
      await lifetimeInput.fill(origLifetime);
      await screen.locator(`input[type="radio"][name="observePolicy"][value="${origPolicy}"]`).check();
      const res = await putted(page, ROUTE_RATE_ALERT_SETTINGS, () => saveBtn.click());
      expect(res.ok, `restore PUT /api/rate-alert-settings: ${res.text}`).toBeTruthy();
      restored = true;
    };

    try {
      await cooldownInput.fill(nextCooldown);
      await maxActiveInput.fill(nextMaxActive);
      await lifetimeInput.fill(nextLifetime);
      await screen.locator(`input[type="radio"][name="observePolicy"][value="${nextPolicy}"]`).check();

      const saved = await putted(page, ROUTE_RATE_ALERT_SETTINGS, () => saveBtn.click());
      expect(saved.ok, `PUT /api/rate-alert-settings: ${saved.text}`).toBeTruthy();
      await expect(screen.locator('.rate-alert-success[role="status"]')).toBeVisible({ timeout: 15_000 });
      await shot(page, 'flow-admin-pricing-rates', '4-rate-alert-settings-saved');

      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      expect(Number(await cooldownInput.inputValue())).toBeCloseTo(Number(nextCooldown), 4);
      await expect(maxActiveInput).toHaveValue(nextMaxActive);
      await expect(lifetimeInput).toHaveValue(nextLifetime);
      await expect(screen.locator(`input[type="radio"][name="observePolicy"][value="${nextPolicy}"]`)).toBeChecked();

      await restoreRateAlertSettings();
    } finally {
      await restoreRateAlertSettings().catch((e) => console.warn('[rate-alert-settings] restore failed:', e));
    }
  });

  test('branchAlertSettingsSave (POST /api/v3/commands/branch_alert_settings_save): saves settings on /#alert-settings, reads back after reload, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/alert-settings/AlertSettings.tsx:148,160,184,193,207,215,224 (menu.ts:12 needs AlertSettingsSave)
    await signIn(page, 'executive', `/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
    const screen = page.locator('.alert-settings-container');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const staleInput = screen.locator('input#stale-data-minutes');
    const addCurrencyInput = screen.locator('input#add-currency');
    const addFloorBtn = screen.locator('button.add-floor-button');
    const saveBtn = screen.locator('.alert-settings-actions button[type="button"]');

    await expect(staleInput).toBeVisible({ timeout: 20_000 });

    const origStaleRaw = (await staleInput.inputValue()).trim();
    const origStale = Number.isInteger(Number(origStaleRaw)) && Number(origStaleRaw) > 0 ? origStaleRaw : '30';

    const floorRows = screen.locator('[data-floor]');
    const floorCount = await floorRows.count();
    const origFloors: Array<{ currency: string; minQuantity: string }> = [];
    for (let i = 0; i < floorCount; i++) {
      const row = floorRows.nth(i);
      const currency = (await row.getAttribute('data-floor')) ?? '';
      const minQuantity = (await row.locator('input[type="text"]').inputValue()).trim();
      if (currency) {
        origFloors.push({ currency, minQuantity: minQuantity || '0' });
      }
    }

    const nextStale = origStale === '45' ? '60' : '45';
    const origUsdFloor = origFloors.find((f) => f.currency === 'USD')?.minQuantity;
    const nextUsdFloor = origUsdFloor === '1500' || origUsdFloor === '1500.0000' ? '2000' : '1500';

    let restored = false;
    const restoreBranchAlertSettings = async () => {
      if (restored) return;
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
      await expect(screen).toBeVisible({ timeout: 20_000 });

      await staleInput.fill(origStale);

      const currentRows = screen.locator('[data-floor]');
      const count = await currentRows.count();
      const currentCodes: string[] = [];
      for (let i = 0; i < count; i++) {
        const code = await currentRows.nth(i).getAttribute('data-floor');
        if (code) currentCodes.push(code);
      }
      for (const code of currentCodes) {
        if (!origFloors.some((f) => f.currency === code)) {
          await screen.locator(`[data-floor="${code}"] button.remove-floor-button`).click();
        }
      }
      for (const f of origFloors) {
        const row = screen.locator(`[data-floor="${f.currency}"]`);
        if ((await row.count()) === 0) {
          await addCurrencyInput.fill(f.currency);
          await addFloorBtn.click();
        }
        await screen.locator(`[data-floor="${f.currency}"] input[type="text"]`).fill(f.minQuantity);
      }

      const res = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(res.ok, `restore branch_alert_settings_save: ${res.text}`).toBeTruthy();
      restored = true;
    };

    try {
      await staleInput.fill(nextStale);
      const usdFloorRow = screen.locator('[data-floor="USD"]');
      if ((await usdFloorRow.count()) === 0) {
        await addCurrencyInput.fill('USD');
        await addFloorBtn.click();
      }
      await screen.locator('[data-floor="USD"] input[type="text"]').fill(nextUsdFloor);

      const saved = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(saved.ok, `POST /api/v3/commands/branch_alert_settings_save: ${saved.text}`).toBeTruthy();
      await shot(page, 'flow-admin-pricing-rates', '5-branch-alert-settings-saved');

      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await expect(staleInput).toHaveValue(nextStale);
      expect(Number(await screen.locator('[data-floor="USD"] input[type="text"]').inputValue())).toBeCloseTo(
        Number(nextUsdFloor),
        4,
      );

      await restoreBranchAlertSettings();
    } finally {
      await restoreBranchAlertSettings().catch((e) => console.warn('[alert-settings] restore failed:', e));
    }
  });

  // ── (b) Invalid inputs (3–5 cases per form, checked in both 'th' and 'en') ─

  test('pricingConfigure and pricingApprove (b): refuse invalid inputs in th and en with exact worded text, preserve typed values, and restore in finally', async ({
    page,
  }) => {
    await signIn(page, 'executive', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
    const initialBoard = await fetchPricingBoard(page, users.branch);
    if (!initialBoard?.policyVersionId || !initialBoard.valueTiersOn || initialBoard.rows.length === 0) {
      const reason = `pricing_board on branch ${users.branch} is not ready (policyVersionId=${initialBoard?.policyVersionId ?? 'null'}, valueTiersOn=${String(initialBoard?.valueTiersOn)}, rows=${initialBoard?.rows.length ?? 0})`;
      console.log(`[qa-r9-6] pricingConfigure (b): skipping — ${reason}`);
      test.skip(true, reason);
      return;
    }

    const policyVersionId = initialBoard.policyVersionId;
    const targetRow = initialBoard.rows.find((r) => r.currency === 'USD') ?? initialBoard.rows[0]!;
    const currency = targetRow.currency;
    const wasOverridden = Boolean(targetRow.overridden);
    const origMinSpread = targetRow.minSpreadPct ?? '';

    await page.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
    await openPricingRow(page, currency);

    const policyForm = page.locator('form.pricing-policy-form');
    await expect(policyForm).toBeVisible({ timeout: 20_000 });

    const origBuyMargins: Record<number, string> = {};
    const origSellMargins: Record<number, string> = {};
    for (const t of [1, 2, 3, 4]) {
      origBuyMargins[t] = (await policyForm.locator(`input[name="buy-margin-${t}"]`).inputValue()).trim() || '0';
      origSellMargins[t] = (await policyForm.locator(`input[name="sell-margin-${t}"]`).inputValue()).trim() || '0';
    }

    const restorePricingInFinally = async () => {
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
      await openPricingRow(page, currency);
      if (!wasOverridden) {
        await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-restore-b-clear`);
        const clearBtn = policyForm.locator('button[name="clearOverride"]');
        const revCfg = await posted(page, ROUTE_PRICING_CONFIGURE, () => clearBtn.click());
        expect(revCfg.ok, `restore clearOverride in (b): ${revCfg.text}`).toBeTruthy();
      } else {
        for (const t of [1, 2, 3, 4]) {
          await policyForm.locator(`input[name="buy-margin-${t}"]`).fill(origBuyMargins[t]);
          await policyForm.locator(`input[name="sell-margin-${t}"]`).fill(origSellMargins[t]);
        }
        await policyForm.locator('input[name="minSpreadPct"]').fill(origMinSpread);
        await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-restore-b-policy`);
        const saveBtn = policyForm.locator('button[name="save"]');
        const revCfg = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(revCfg.ok, `restore pricingConfigure in (b): ${revCfg.text}`).toBeTruthy();
      }
    };

    try {
      for (const lang of STAFF_TH_EN) {
        await withLang(page, lang);
        await page.goto('about:blank');
        await page.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
        await openPricingRow(page, currency);

        const i18n = PRICING_I18N[lang];
        const policyAlert = policyForm.locator('[role="alert"].pricing-policy-alert');
        const saveBtn = policyForm.locator('button[name="save"]');
        const clearBtn = policyForm.locator('button[name="clearOverride"]');

        // Verify screen i18n labels in th and en (web/src/screens/admin/pricing/i18n.ts:21-35,76-90; PricingPolicy.tsx:197-244).
        await expect(policyForm.locator('label[for="minSpreadPct"]')).toHaveText(i18n.minSpread);
        await expect(policyForm.locator('label[for="reason"]')).toHaveText(i18n.reason);
        await expect(saveBtn).toHaveText(i18n.save);
        await expect(clearBtn).toHaveText(i18n.cancel);

        // TierEditor i18n state when suggestion is null (#4324, TierEditor.tsx:105,193,196).
        const tierEditor = page.locator('section.tier-editor');
        await expect(tierEditor.locator('.tier-editor-empty')).toHaveText(i18n.noSuggestion);
        await expect(tierEditor.locator('button[name="approve"]')).toHaveText(i18n.approve);
        await expect(tierEditor.locator('button[name="approve"]')).toBeDisabled();

        // Case 1: empty reason ("") -> 400 validationFailed (pricing_configure.rs:100-106)
        await policyForm.locator('input[name="minSpreadPct"]').fill('0.25');
        await policyForm.locator('input[name="reason"]').fill('');
        const emptyReasonRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(emptyReasonRes.ok).toBeFalsy();
        expect(emptyReasonRes.text).toMatch(/^400\b/);
        await expect(policyAlert).toHaveText(PRICING_CONFIGURE_REFUSALS.emptyReason);
        await expect(policyForm.locator('input[name="minSpreadPct"]')).toHaveValue('0.25');

        // Case 2: negative minSpreadPct ("-0.5") -> 400 validationFailed (currency_tier_override.rs:61-68)
        await policyForm.locator('input[name="minSpreadPct"]').fill('-0.5');
        await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-bad-spread-neg`);
        const negSpreadRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(negSpreadRes.ok).toBeFalsy();
        expect(negSpreadRes.text).toMatch(/^400\b/);
        await expect(policyAlert).toHaveText(PRICING_CONFIGURE_REFUSALS.negativeSpread);
        await expect(policyForm.locator('input[name="minSpreadPct"]')).toHaveValue('-0.5');

        // Case 3: out-of-range minSpreadPct ("6.5", > 5) -> 400 validationFailed (currency_tier_override.rs:61-68)
        await policyForm.locator('input[name="minSpreadPct"]').fill('6.5');
        const highSpreadRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(highSpreadRes.ok).toBeFalsy();
        expect(highSpreadRes.text).toMatch(/^400\b/);
        await expect(policyAlert).toHaveText(PRICING_CONFIGURE_REFUSALS.highSpread);
        await expect(policyForm.locator('input[name="minSpreadPct"]')).toHaveValue('6.5');
        await policyForm.locator('input[name="minSpreadPct"]').fill(origMinSpread);

        // Case 4: too many decimals on tier buy margin ("0.12345", scale > 4) -> 400 validationFailed (rate_percent.rs:17-24)
        await policyForm.locator('input[name="buy-margin-1"]').fill('0.12345');
        const badScaleRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(badScaleRes.ok).toBeFalsy();
        expect(badScaleRes.text).toMatch(/^400\b/);
        await expect(policyAlert).toHaveText(PRICING_CONFIGURE_REFUSALS.tooManyDecimals);
        await expect(policyForm.locator('input[name="buy-margin-1"]')).toHaveValue('0.12345');

        // Case 5: out-of-bound tier margin ("1000000", exceeds numeric(9,4)) -> 400 validationFailed (rate_percent.rs:17-24)
        await policyForm.locator('input[name="buy-margin-1"]').fill('1000000');
        const invalidMarginRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
        expect(invalidMarginRes.ok).toBeFalsy();
        expect(invalidMarginRes.text).toMatch(/^400\b/);
        await expect(policyAlert).toHaveText(PRICING_CONFIGURE_REFUSALS.invalidMargin);
        await expect(policyForm.locator('input[name="buy-margin-1"]')).toHaveValue('1000000');
        await policyForm.locator('input[name="buy-margin-1"]').fill(origBuyMargins[1]);
      }

      // pricingApprove (b): 4 invalid input cases against POST /api/pricing/approve (pricing_approve.rs:124-218).
      // (TierEditor's approve button is disabled on main until #4324 populates pricing_board.suggestion.)
      const sameOrigin = sameOriginHeaders();
      const approveUrl = `/api/pricing/approve?branch=${encodeURIComponent(users.branch)}`;
      const validTiers = [
        { tierNo: 1, buy: '32.10', sell: '32.60' },
        { tierNo: 2, buy: '32.12', sell: '32.58' },
        { tierNo: 3, buy: '32.14', sell: '32.56' },
        { tierNo: 4, buy: '32.16', sell: '32.54' },
      ];
      const approveInvalidCases: Array<{
        label: string;
        data: Record<string, unknown>;
        expectedMessage: string;
      }> = [
        {
          label: 'empty reason',
          data: { policyVersionId, rows: [{ currency, tiers: validTiers }], reason: '' },
          expectedMessage: PRICING_APPROVE_REFUSALS.emptyReason,
        },
        {
          label: 'empty rows',
          data: { policyVersionId, rows: [], reason: `${RUN_MARKER}-empty-rows` },
          expectedMessage: PRICING_APPROVE_REFUSALS.emptyRows,
        },
        {
          label: '3 tiers instead of 4',
          data: {
            policyVersionId,
            rows: [{ currency, tiers: validTiers.slice(0, 3) }],
            reason: `${RUN_MARKER}-three-tiers`,
          },
          expectedMessage: PRICING_APPROVE_REFUSALS.wrongTierCount,
        },
        {
          label: 'negative tier buy rate',
          data: {
            policyVersionId,
            rows: [
              {
                currency,
                tiers: [{ tierNo: 1, buy: '-10', sell: '32.60' }, ...validTiers.slice(1)],
              },
            ],
            reason: `${RUN_MARKER}-neg-buy`,
          },
          expectedMessage: PRICING_APPROVE_REFUSALS.invalidBuyRate,
        },
      ];

      for (const c of approveInvalidCases) {
        const res = await page.request.post(approveUrl, {
          headers: { ...sameOrigin, 'idempotency-key': crypto.randomUUID() },
          data: c.data,
        });
        expect(res.status(), `pricingApprove (${c.label}) status`).toBe(400);
        const body = (await res.json()) as { error?: { code?: string; message?: string } };
        expect(body.error?.code, `pricingApprove (${c.label}) code`).toBe('validationFailed');
        expect(body.error?.message, `pricingApprove (${c.label}) message`).toBe(c.expectedMessage);
      }
    } finally {
      await restorePricingInFinally().catch((e) => console.warn('[pricingConfigure (b)] restore failed:', e));
    }
  });

  test('orgStartingPriceSave (b): refuses invalid inputs in th and en with exact worded text, preserves typed values, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/starting-prices/StartingPrices.tsx:186-277, i18n.ts:34-44,78-88
    await signIn(page, 'executive', '/#starting-prices');
    const screen = page.locator('.xc-sp-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const usdRow = screen.locator('tr.xc-sp-currency-row[data-currency="USD"]');
    if ((await usdRow.count()) === 0) {
      await screen.locator('input.xc-sp-add-code-input').fill('USD');
      await screen.locator('button.xc-sp-add-code-btn').click();
    }
    await expect(usdRow).toBeVisible({ timeout: 20_000 });

    const origBuyRaw = (await usdRow.locator('input.xc-sp-input[data-field="buy"]').inputValue()).trim();
    const origSellRaw = (await usdRow.locator('input.xc-sp-input[data-field="sell"]').inputValue()).trim();
    const origBuy = Number(origBuyRaw) > 0 ? origBuyRaw : '32.10';
    const origSell = Number(origSellRaw) > Number(origBuy) ? origSellRaw : '32.60';

    try {
      for (const lang of STAFF_TH_EN) {
        await withLang(page, lang);
        await page.goto('about:blank');
        await page.goto('/#starting-prices');
        await expect(screen).toBeVisible({ timeout: 20_000 });

        const i18n = STARTING_PRICES_I18N[lang];
        if ((await usdRow.count()) === 0) {
          await screen.locator('input.xc-sp-add-code-input').fill('USD');
          await screen.locator('button.xc-sp-add-code-btn').click();
        }
        const buyInput = usdRow.locator('input.xc-sp-input[data-field="buy"]');
        const sellInput = usdRow.locator('input.xc-sp-input[data-field="sell"]');
        const reasonInput = screen.locator('input#xc-sp-reason[name="reason"]');
        const saveBtn = screen.locator('button.xc-sp-save-btn[data-action="save"]');

        const countSavePosts = async (act: () => Promise<void>) => {
          let posts = 0;
          const onReq = (req: { method(): string; url(): string }) => {
            if (req.method() === 'POST' && ROUTE_ORG_STARTING_PRICE_SAVE.test(new URL(req.url()).pathname)) {
              posts++;
            }
          };
          page.on('request', onReq);
          try {
            await act();
          } finally {
            page.off('request', onReq);
          }
          return posts;
        };

        // Case 1: empty reason -> .xc-sp-reason-error
        await buyInput.fill('32.10');
        await sellInput.fill('32.60');
        await reasonInput.fill('');
        expect(await countSavePosts(() => saveBtn.click())).toBe(0);
        await expect(screen.locator('.xc-sp-reason-error')).toBeVisible();
        await expect(screen.locator('.xc-sp-reason-error')).toHaveText(i18n.reasonRequired);
        await expect(buyInput).toHaveValue('32.10');
        await expect(sellInput).toHaveValue('32.60');

        // Case 2: negative buy price ("-5") -> .xc-sp-buy-error
        await buyInput.fill('-5');
        await sellInput.fill('32.60');
        await reasonInput.fill(`${RUN_MARKER}-neg-buy`);
        expect(await countSavePosts(() => saveBtn.click())).toBe(0);
        await expect(usdRow.locator('.xc-sp-buy-error[data-error-for="buy"]')).toBeVisible();
        await expect(usdRow.locator('.xc-sp-buy-error[data-error-for="buy"]')).toHaveText(i18n.positiveDecimal);
        await expect(buyInput).toHaveValue('-5');

        // Case 3: zero sell price ("0") -> .xc-sp-sell-error
        await buyInput.fill('32.10');
        await sellInput.fill('0');
        expect(await countSavePosts(() => saveBtn.click())).toBe(0);
        await expect(usdRow.locator('.xc-sp-sell-error[data-error-for="sell"]')).toBeVisible();
        await expect(usdRow.locator('.xc-sp-sell-error[data-error-for="sell"]')).toHaveText(i18n.positiveDecimal);
        await expect(sellInput).toHaveValue('0');

        // Case 4: buy > sell ("40.00" > "30.00") -> .xc-sp-row-error
        await buyInput.fill('40.00');
        await sellInput.fill('30.00');
        expect(await countSavePosts(() => saveBtn.click())).toBe(0);
        await expect(usdRow.locator('.xc-sp-row-error.xc-sp-currency-error')).toBeVisible();
        await expect(usdRow.locator('.xc-sp-row-error.xc-sp-currency-error')).toHaveText(i18n.buyAboveSell);
        await expect(buyInput).toHaveValue('40.00');
        await expect(sellInput).toHaveValue('30.00');

        // Case 5: too many decimals ("32.123456789", >8 decimals) -> .xc-sp-buy-error, no POST
        await buyInput.fill('32.123456789');
        await sellInput.fill('32.60');
        expect(await countSavePosts(() => saveBtn.click())).toBe(0);
        await expect(usdRow.locator('.xc-sp-buy-error[data-error-for="buy"]')).toBeVisible();
        await expect(usdRow.locator('.xc-sp-buy-error[data-error-for="buy"]')).toHaveText(i18n.positiveDecimal);
        await expect(buyInput).toHaveValue('32.123456789');
      }
    } finally {
      await page.goto('/#starting-prices').catch(() => {});
      await usdRow.locator('input.xc-sp-input[data-field="buy"]').fill(origBuy).catch(() => {});
      await usdRow.locator('input.xc-sp-input[data-field="sell"]').fill(origSell).catch(() => {});
      await screen.locator('input#xc-sp-reason[name="reason"]').fill(`${RUN_MARKER}-restore-sp-b`).catch(() => {});
      await posted(page, ROUTE_ORG_STARTING_PRICE_SAVE, () =>
        screen.locator('button.xc-sp-save-btn[data-action="save"]').click(),
      ).catch(() => {});
    }
  });

  test('rateAlertSettingsSave (b): refuses invalid inputs in th and en with exact worded text, preserves typed values, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:142-178, i18n.ts:34-38,74-78
    await signIn(page, 'executive', '/#rate-alert-settings');
    const screen = page.locator('.rate-alert-settings');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const cooldownInput = screen.locator('input#default-cooldown-hours[name="defaultCooldownHours"]');
    const maxActiveInput = screen.locator('input#max-active-per-customer[name="maxActivePerCustomer"]');
    const lifetimeInput = screen.locator('input#default-lifetime-days[name="defaultLifetimeDays"]');
    const saveBtn = screen.locator('button.rate-alert-submit-btn[type="submit"]');

    const origCooldownRaw = (await cooldownInput.inputValue()).trim();
    const origMaxActiveRaw = (await maxActiveInput.inputValue()).trim();
    const origLifetimeRaw = (await lifetimeInput.inputValue()).trim();
    const origCooldown = Number(origCooldownRaw) > 0 ? origCooldownRaw : '24';
    const origMaxActive =
      Number.isInteger(Number(origMaxActiveRaw)) && Number(origMaxActiveRaw) > 0 ? origMaxActiveRaw : '10';
    const origLifetime =
      origLifetimeRaw === '' || (Number.isInteger(Number(origLifetimeRaw)) && Number(origLifetimeRaw) > 0)
        ? origLifetimeRaw
        : '30';

    try {
      for (const lang of STAFF_TH_EN) {
        await withLang(page, lang);
        await page.goto('about:blank');
        await page.goto('/#rate-alert-settings');
        await expect(screen).toBeVisible({ timeout: 20_000 });

        const i18n = RATE_ALERT_SETTINGS_I18N[lang];

        // Case 1: zero cooldown ("0") -> [data-error="cooldown"]
        await cooldownInput.fill('0');
        await maxActiveInput.fill('10');
        await lifetimeInput.fill('30');
        await saveBtn.click();
        await expect(screen.locator('[data-error="cooldown"]')).toHaveText(i18n.errorCooldown);
        await expect(cooldownInput).toHaveValue('0');

        // Case 2: negative cooldown ("-2") -> [data-error="cooldown"]
        await cooldownInput.fill('-2');
        await saveBtn.click();
        await expect(screen.locator('[data-error="cooldown"]')).toHaveText(i18n.errorCooldown);
        await expect(cooldownInput).toHaveValue('-2');

        // Case 3: non-integer maxActivePerCustomer ("2.5") -> [data-error="maxActive"]
        await cooldownInput.fill('24');
        await maxActiveInput.fill('2.5');
        await saveBtn.click();
        await expect(screen.locator('[data-error="maxActive"]')).toHaveText(i18n.errorMaxActive);
        await expect(maxActiveInput).toHaveValue('2.5');

        // Case 4: negative defaultLifetimeDays ("-5") -> [data-error="lifetime"]
        await maxActiveInput.fill('10');
        await lifetimeInput.fill('-5');
        await saveBtn.click();
        await expect(screen.locator('[data-error="lifetime"]')).toHaveText(i18n.errorLifetime);
        await expect(lifetimeInput).toHaveValue('-5');
      }
    } finally {
      await cooldownInput.fill(origCooldown).catch(() => {});
      await maxActiveInput.fill(origMaxActive).catch(() => {});
      await lifetimeInput.fill(origLifetime).catch(() => {});
    }
  });

  test('branchAlertSettingsSave (b): refuses invalid inputs in th and en with exact worded text, preserves typed values, and restores in finally', async ({
    page,
  }) => {
    // web/src/screens/admin/alert-settings/AlertSettings.tsx:148, branch_alert_settings_save.rs:60-105
    await signIn(page, 'executive', `/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
    const screen = page.locator('.alert-settings-container');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    const staleInput = screen.locator('input#stale-data-minutes');
    const addCurrencyInput = screen.locator('input#add-currency');
    const addFloorBtn = screen.locator('button.add-floor-button');
    const saveBtn = screen.locator('.alert-settings-actions button[type="button"]');
    const errorAlert = screen.locator('.alert-settings-error[role="alert"]');

    const origStaleRaw = (await staleInput.inputValue()).trim();
    const origStale = Number.isInteger(Number(origStaleRaw)) && Number(origStaleRaw) > 0 ? origStaleRaw : '30';
    const floorRows = screen.locator('[data-floor]');
    const floorCount = await floorRows.count();
    const origFloors: Array<{ currency: string; minQuantity: string }> = [];
    for (let i = 0; i < floorCount; i++) {
      const row = floorRows.nth(i);
      const currency = (await row.getAttribute('data-floor')) ?? '';
      const minQuantity = (await row.locator('input[type="text"]').inputValue()).trim();
      if (currency) {
        origFloors.push({ currency, minQuantity: minQuantity || '0' });
      }
    }

    const restoreBranchAlertInFinally = async () => {
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await staleInput.fill(origStale);

      const currentRows = screen.locator('[data-floor]');
      const count = await currentRows.count();
      const currentCodes: string[] = [];
      for (let i = 0; i < count; i++) {
        const code = await currentRows.nth(i).getAttribute('data-floor');
        if (code) currentCodes.push(code);
      }
      for (const code of currentCodes) {
        if (!origFloors.some((f) => f.currency === code)) {
          await screen.locator(`[data-floor="${code}"] button.remove-floor-button`).click();
        }
      }
      for (const f of origFloors) {
        const row = screen.locator(`[data-floor="${f.currency}"]`);
        if ((await row.count()) === 0) {
          await addCurrencyInput.fill(f.currency);
          await addFloorBtn.click();
        }
        await screen.locator(`[data-floor="${f.currency}"] input[type="text"]`).fill(f.minQuantity);
      }
      const res = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(res.ok, `restore branch_alert_settings_save in (b): ${res.text}`).toBeTruthy();
    };

    try {
      for (const lang of STAFF_TH_EN) {
        await withLang(page, lang);
        await page.goto('about:blank');
        await page.goto(`/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
        await expect(screen).toBeVisible({ timeout: 20_000 });

        const i18n = ALERT_SETTINGS_I18N[lang];
        // Verify screen i18n labels in th and en (web/src/screens/admin/alert-settings/i18n.ts:2-14,19-31; AlertSettings.tsx:157-230).
        await expect(screen.locator('label[for="stale-data-minutes"]')).toContainText(i18n.staleDataMinutes);
        await expect(screen.locator('.floors-section h3')).toHaveText(i18n.floorsHeading);
        await expect(screen.locator('label[for="add-currency"]')).toContainText(i18n.addCurrency);
        await expect(addFloorBtn).toHaveText(i18n.addFloor);
        await expect(saveBtn).toHaveText(i18n.save);

        // Case 1: zero staleDataMinutes ("0") -> refused at field (AlertSettings.tsx:158), no POST
        await staleInput.fill('0');
        let zeroPosts = 0;
        const onZeroReq = (req: { method(): string; url(): string }) => {
          if (req.method() === 'POST' && ROUTE_BRANCH_ALERT_SETTINGS_SAVE.test(new URL(req.url()).pathname)) {
            zeroPosts++;
          }
        };
        page.on('request', onZeroReq);
        try {
          await saveBtn.click();
        } finally {
          page.off('request', onZeroReq);
        }
        expect(zeroPosts, `case 1 zero staleDataMinutes sends no POST (${lang})`).toBe(0);
        const minutesErr = screen.locator('.alert-settings-field .alert-settings-error');
        await expect(minutesErr, `case 1 worded field error (${lang})`).toBeVisible({ timeout: 10_000 });
        await expect(minutesErr, `case 1 worded refusal (${lang})`).toHaveText(STALE_MINUTES_ERR[lang]);
        await expect(staleInput).toHaveValue('0');

        // Case 2: negative staleDataMinutes ("-10") -> refused at field (AlertSettings.tsx:155), no POST
        await staleInput.fill('-10');
        let negPosts = 0;
        const onNegReq = (req: { method(): string; url(): string }) => {
          if (req.method() === 'POST' && ROUTE_BRANCH_ALERT_SETTINGS_SAVE.test(new URL(req.url()).pathname)) {
            negPosts++;
          }
        };
        page.on('request', onNegReq);
        try {
          await saveBtn.click();
        } finally {
          page.off('request', onNegReq);
        }
        expect(negPosts, `case 2 negative staleDataMinutes sends no POST (${lang})`).toBe(0);
        await expect(minutesErr, `case 2 worded field error (${lang})`).toBeVisible({ timeout: 10_000 });
        await expect(minutesErr, `case 2 worded refusal (${lang})`).toHaveText(STALE_MINUTES_ERR[lang]);
        await expect(staleInput).toHaveValue('-10');
        await staleInput.fill(origStale);

        // Ensure USD floor row is present for floor input validation cases
        if ((await screen.locator('[data-floor="USD"]').count()) === 0) {
          await addCurrencyInput.fill('USD');
          await addFloorBtn.click();
        }
        const usdFloorInput = screen.locator('[data-floor="USD"] input[type="text"]');
        await expect(screen.locator('[data-floor="USD"] button.remove-floor-button')).toHaveText(i18n.removeFloor);

        // Case 3: negative floor minQuantity ("-50") -> refused at field (AlertSettings.tsx:167), no POST
        await usdFloorInput.fill('-50');
        let negFloorPosts = 0;
        const onNegFloorReq = (req: { method(): string; url(): string }) => {
          if (req.method() === 'POST' && ROUTE_BRANCH_ALERT_SETTINGS_SAVE.test(new URL(req.url()).pathname)) {
            negFloorPosts++;
          }
        };
        page.on('request', onNegFloorReq);
        try {
          await saveBtn.click();
        } finally {
          page.off('request', onNegFloorReq);
        }
        expect(negFloorPosts, `case 3 negative floor minQuantity sends no POST (${lang})`).toBe(0);
        const usdFloorErr = screen.locator('[data-floor="USD"] .alert-settings-error');
        await expect(usdFloorErr, `case 3 floor worded error (${lang})`).toBeVisible({ timeout: 10_000 });
        await expect(usdFloorErr, `case 3 floor worded refusal (${lang})`).toHaveText(FLOOR_QUANTITY_ERR[lang]);
        await expect(usdFloorInput).toHaveValue('-50');

        // Case 4: too many decimals on floor minQuantity ("100.12345", scale > 4) -> refused at field (AlertSettings.tsx:167), no POST
        await usdFloorInput.fill('100.12345');
        let scaleFloorPosts = 0;
        const onScaleFloorReq = (req: { method(): string; url(): string }) => {
          if (req.method() === 'POST' && ROUTE_BRANCH_ALERT_SETTINGS_SAVE.test(new URL(req.url()).pathname)) {
            scaleFloorPosts++;
          }
        };
        page.on('request', onScaleFloorReq);
        try {
          await saveBtn.click();
        } finally {
          page.off('request', onScaleFloorReq);
        }
        expect(scaleFloorPosts, `case 4 too many decimals floor minQuantity sends no POST (${lang})`).toBe(0);
        await expect(usdFloorErr, `case 4 floor worded error (${lang})`).toBeVisible({ timeout: 10_000 });
        await expect(usdFloorErr, `case 4 floor worded refusal (${lang})`).toHaveText(FLOOR_QUANTITY_ERR[lang]);
        await expect(usdFloorInput).toHaveValue('100.12345');
        await usdFloorInput.fill(origFloors.find((f) => f.currency === 'USD')?.minQuantity ?? '1000');

        // Case 5: unknown floor currency ("ZZZ" with minQuantity "10") passes client check (/^[A-Z]{3}$/, valid decimal);
        // refused by server -> 400 validationFailed (branch_alert_settings_save.rs:96-102)
        await addCurrencyInput.fill('ZZZ');
        await addFloorBtn.click();
        const zzzFloorInput = screen.locator('[data-floor="ZZZ"] input[type="text"]');
        await zzzFloorInput.fill('10');
        const unknownCurRes = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
        expect(unknownCurRes.ok).toBeFalsy();
        expect(unknownCurRes.text).toMatch(/^400\b/);
        await expect(errorAlert).toHaveText(BRANCH_ALERT_REFUSALS.unknownCurrency('ZZZ'));
        await expect(zzzFloorInput).toHaveValue('10');
        await screen.locator('[data-floor="ZZZ"] button.remove-floor-button').click();
      }
    } finally {
      await restoreBranchAlertInFinally().catch((e) => console.warn('[branchAlertSettingsSave (b)] restore failed:', e));
    }
  });

  // ── (c) Permission ────────────────────────────────────────────────────────

  test('teller sees none of the 4 menus (smoke-rbac-matrix.spec.ts) and direct writes with same-origin headers are refused with roleNotAllowed|outOfScope', async ({
    page,
  }) => {
    // Menu visibility and refused-hash fallback are also covered across all staff roles in
    // qa/e2e/tests/smoke-rbac-matrix.spec.ts:41-69,120-170.
    test.skip(!hasAccount('teller'), 'no teller account in users.json: run scripts/reseed.sh');
    await signIn(page, 'teller', '/');

    const perms = await mePermissions(page);
    expect(perms).not.toContain('ReferenceRatesRead');
    expect(perms).not.toContain('RatePolicySave');
    expect(perms).not.toContain('RatesDraft');
    expect(perms).not.toContain('OrgStartingPricesManage');
    expect(perms).not.toContain('RateAlertSettingsSave');
    expect(perms).not.toContain('AlertSettingsSave');

    await page.goto('/');
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });

    const gatedMenus = ['pricing', 'starting-prices', 'rate-alert-settings', 'alert-settings'] as const;
    for (const id of gatedMenus) {
      await expect(
        page.locator(`nav.admin-nav a[href="#${id}"]`),
        `teller lacks menu link #${id} (smoke-rbac-matrix.spec.ts:148)`,
      ).toHaveCount(0);
    }

    // Same-origin headers required by api/src/presentation/extractors/origin.rs:33 and command.rs:256
    // (see flow-desk-kiosk-cancel.spec.ts:59) so requests reach Policy::authorize instead of 400 originRejected.
    const sameOrigin = sameOriginHeaders();
    const branch = encodeURIComponent(users.branch);

    const checks: Array<{
      name: string;
      method: 'POST' | 'PUT';
      url: string;
      data: Record<string, unknown>;
    }> = [
      {
        name: 'pricingConfigure',
        method: 'POST',
        url: `/api/pricing/configure?branch=${branch}`,
        data: {
          policyVersionId: crypto.randomUUID(),
          currency: 'USD',
          tiers: null,
          minSpreadPct: '-1',
          reason: '',
        },
      },
      {
        name: 'pricingApprove',
        method: 'POST',
        url: `/api/pricing/approve?branch=${branch}`,
        data: {
          policyVersionId: crypto.randomUUID(),
          rows: [],
          reason: '',
        },
      },
      {
        name: 'orgStartingPriceSave',
        method: 'POST',
        url: '/api/v3/commands/org_starting_price_save',
        data: {
          entries: [],
          reason: '',
        },
      },
      {
        name: 'rateAlertSettingsSave',
        method: 'PUT',
        url: '/api/rate-alert-settings',
        data: {
          defaultCooldownSeconds: -1,
          maxActivePerCustomer: 0,
          defaultLifetimeDays: -1,
          observePolicy: 'ALL_FRESH',
        },
      },
      {
        name: 'branchAlertSettingsSave',
        method: 'POST',
        url: `/api/v3/commands/branch_alert_settings_save?branch=${branch}`,
        data: {
          branch: users.branch,
          staleDataMinutes: 0,
          floors: [],
        },
      },
    ];

    for (const c of checks) {
      const key = `${RUN_MARKER}-teller-${c.name}`;
      const res =
        c.method === 'PUT'
          ? await page.request.put(c.url, { headers: { ...sameOrigin, 'Idempotency-Key': key }, data: c.data })
          : await page.request.post(c.url, { headers: { ...sameOrigin, 'Idempotency-Key': key }, data: c.data });
      const body = await res.text();
      console.log(`[R9-6 teller] ${c.method} ${c.url} -> ${res.status()} ${body.slice(0, 200)}`);
      expect(res.ok(), `${c.name} must be refused for teller: ${body}`).toBeFalsy();
      expect([400, 403], `${c.name} refusal status for teller: ${body}`).toContain(res.status());
      expect(body, `${c.name} refusal code`).toMatch(/roleNotAllowed|outOfScope/);
    }
  });
});
