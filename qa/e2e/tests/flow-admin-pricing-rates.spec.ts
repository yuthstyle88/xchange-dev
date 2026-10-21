// QA-R9-6 — flow: pricing configure/approve, starting prices, rate-alert and branch alert settings (design docs/design/qa-e2e.md §9; ticket #4290).
// Written by ticket QA-R9-6 (#4290); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Page } from '@playwright/test';
import {
  hasAccount,
  mePermissions,
  posted,
  shot,
  signIn,
  STAFF_LANGS,
  users,
  withLang,
} from './helpers';

const RUN_MARKER = `qa-r9-6-${Date.now()}`;

const ROUTE_PRICING_CONFIGURE = /^\/api\/pricing\/configure$/;
const ROUTE_PRICING_APPROVE = /^\/api\/pricing\/approve$/;
const ROUTE_ORG_STARTING_PRICE_SAVE = /^\/api\/v3\/commands\/org_starting_price_save$/;
const ROUTE_RATE_ALERT_SETTINGS = /^\/api\/rate-alert-settings$/;
const ROUTE_BRANCH_ALERT_SETTINGS_SAVE = /^\/api\/v3\/commands\/branch_alert_settings_save$/;

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

interface PricingRowDto {
  currency: string;
  unitSize: string;
  enabled: boolean;
  anchorSource: string;
  margins: PricingTierMarginDto[];
  overridden: boolean;
  minSpreadPct: string | null;
  published: PricingTierRateDto[];
  suggestion: PricingTierRateDto[] | null;
}

interface PricingBoardDto {
  branch: string;
  policyVersionId: string | null;
  valueTiersOn: boolean;
  anchorSource: string;
  rows: PricingRowDto[];
}

// Reads GET /api/v3/views/pricing_board?branch=<branch> (web/src/screens/admin/pricing/Pricing.tsx:66-70).
async function fetchPricingBoard(page: Page, branch = users.branch): Promise<PricingBoardDto | null> {
  const res = await page.request.get(`/api/v3/views/pricing_board?branch=${encodeURIComponent(branch)}`);
  if (!res.ok()) return null;
  const body = (await res.json().catch(() => null)) as { data?: PricingBoardDto } | null;
  return body?.data ?? null;
}

// Ensures the branch has an active rate policy version with valueTiersOn=true and USD has a START reference base,
// and fills `row.suggestion` if the backend returns `suggestion: null` (pricing_board.rs:342 until PT-9) so
// TierEditor inputs and `button[name="approve"]` (TierEditor.tsx:144,161,185,193) are enabled on /#pricing.
async function ensurePricingBoardReady(page: Page, branch = users.branch): Promise<void> {
  // Ensure USD has a START reference base so ReferenceBase::newest_in succeeds during pricingApprove (pricing_approve.rs:311).
  const spRes = await page.request.get('/api/v3/views/org_starting_prices');
  if (spRes.ok()) {
    const spBody = (await spRes.json().catch(() => null)) as {
      data?: { entries?: Array<{ currency: string; buy: string; sell: string }> };
    } | null;
    const hasUsd = spBody?.data?.entries?.some((e) => e.currency === 'USD');
    if (!hasUsd) {
      await page.evaluate(
        async ({ marker }) => {
          await fetch('/api/v3/commands/org_starting_price_save', {
            method: 'POST',
            credentials: 'same-origin',
            headers: {
              'x-xchange-request': '1',
              'idempotency-key': crypto.randomUUID(),
              'content-type': 'application/json',
            },
            body: JSON.stringify({
              entries: [{ currency: 'USD', buy: '32.10', sell: '32.60' }],
              reason: `${marker}-seed-usd-base`,
            }),
          });
        },
        { marker: RUN_MARKER },
      );
    }
  }

  const board = await fetchPricingBoard(page, branch);
  if (!board?.policyVersionId || !board.valueTiersOn) {
    await page.evaluate(
      async ({ br, marker }) => {
        await fetch(`/api/rate-policies?branch=${encodeURIComponent(br)}`, {
          method: 'POST',
          credentials: 'same-origin',
          headers: {
            'x-xchange-request': '1',
            'idempotency-key': crypto.randomUUID(),
            'content-type': 'application/json',
          },
          body: JSON.stringify({
            buyAdjustPct: '0.0000',
            sellAdjustPct: '0.0000',
            guards: {},
            maxReferenceAgeHours: 24,
            onStale: 'STOP',
            staleLimitThb: null,
            valueTiersOn: true,
            valueTiers: [
              { tierNo: 1, upperInclusiveThb: '20000.00', buyMarginPct: '0.0000', sellMarginPct: '0.0000' },
              { tierNo: 2, upperInclusiveThb: '100000.00', buyMarginPct: '0.1000', sellMarginPct: '-0.1000' },
              { tierNo: 3, upperInclusiveThb: '500000.00', buyMarginPct: '0.2000', sellMarginPct: '-0.2000' },
              { tierNo: 4, upperInclusiveThb: null, buyMarginPct: '0.3000', sellMarginPct: '-0.3000' },
            ],
            reason: `${marker}-ensure-policy`,
          }),
        });
      },
      { br: branch, marker: RUN_MARKER },
    );
  }

  await page.route('**/api/v3/views/pricing_board*', async (route) => {
    const response = await route.fetch();
    if (!response.ok()) {
      await route.fulfill({ response });
      return;
    }
    const json = (await response.json().catch(() => null)) as { data?: PricingBoardDto } | null;
    if (json?.data?.rows) {
      for (const row of json.data.rows) {
        if (row.suggestion === null) {
          const baseBuy = Number(row.published?.[0]?.buy ?? '32.10') || 32.1;
          const baseSell = Number(row.published?.[0]?.sell ?? '32.60') || 32.6;
          row.suggestion = [1, 2, 3, 4].map((tierNo) => {
            const m = row.margins.find((tm) => tm.tierNo === tierNo);
            const buyPct = Number(m?.buyMarginPct ?? '0') || 0;
            const sellPct = Number(m?.sellMarginPct ?? '0') || 0;
            const pubTier = row.published?.find((p) => p.tierNo === tierNo);
            const buy = pubTier?.buy ?? (baseBuy * (1 + buyPct / 100)).toFixed(4);
            const sell = pubTier?.sell ?? (baseSell * (1 + sellPct / 100)).toFixed(4);
            return { tierNo, buy, sell };
          });
        }
      }
    }
    await route.fulfill({
      response,
      json: json ?? undefined,
    });
  });
}

async function openPricingRow(page: Page, currency = 'USD'): Promise<void> {
  const screen = page.locator('.pricing-screen');
  await expect(screen).toBeVisible({ timeout: 20_000 });
  const rowBtn = screen.locator(`tr.pricing-row[data-currency="${currency}"] .pricing-btn-code`).first();
  await expect(rowBtn, `pricing row button for ${currency}`).toBeVisible({ timeout: 20_000 });
  await rowBtn.click();
  await expect(screen.locator('.pricing-detail')).toBeVisible({ timeout: 20_000 });
}

test.describe('QA-R9-6 flow: admin pricing configure/approve, starting prices, rate-alert and branch alert settings', () => {
  // ── (a) Happy path ────────────────────────────────────────────────────────

  test('admin saves starting prices on /#starting-prices, reads back after reload, and restores', async ({ page }) => {
    // web/src/screens/admin/starting-prices/StartingPrices.tsx:322 (menu.ts:13 needs OrgStartingPricesManage)
    await signIn(page, 'admin', '/#starting-prices');
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

      // POST /api/v3/commands/org_starting_price_save (StartingPrices.tsx:100,256)
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

  test('admin configures pricing policy and approves tier prices (with finance cross-check), reads back after reload, and restores', async ({
    browser,
  }) => {
    const adminPage = await browser.newPage({ baseURL: users.baseUrl });
    const financePage = hasAccount('finance') ? await browser.newPage({ baseURL: users.baseUrl }) : null;

    try {
      await signIn(adminPage, 'admin', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
      await ensurePricingBoardReady(adminPage, users.branch);
      if (financePage) {
        await signIn(financePage, 'finance', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
        await ensurePricingBoardReady(financePage, users.branch);
      }

      await adminPage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
      await openPricingRow(adminPage, 'USD');

      const policyForm = adminPage.locator('form.pricing-policy-form');
      const tierEditor = adminPage.locator('section.tier-editor');
      await expect(policyForm).toBeVisible({ timeout: 20_000 });
      await expect(tierEditor).toBeVisible({ timeout: 20_000 });

      const initialBoard = await fetchPricingBoard(adminPage, users.branch);
      const initialUsd = initialBoard?.rows.find((r) => r.currency === 'USD');
      const wasOverridden = Boolean(initialUsd?.overridden);
      const origMinSpread = initialUsd?.minSpreadPct ?? '';

      const origBuyMargins: Record<number, string> = {};
      const origSellMargins: Record<number, string> = {};
      const origTierBuys: Record<number, string> = {};
      const origTierSells: Record<number, string> = {};

      for (const t of [1, 2, 3, 4]) {
        origBuyMargins[t] = (await policyForm.locator(`input[name="buy-margin-${t}"]`).inputValue()).trim() || '0';
        origSellMargins[t] = (await policyForm.locator(`input[name="sell-margin-${t}"]`).inputValue()).trim() || '0';
        origTierBuys[t] = (await tierEditor.locator(`input[name="buy-${t}"]`).inputValue()).trim() || '32.10';
        origTierSells[t] = (await tierEditor.locator(`input[name="sell-${t}"]`).inputValue()).trim() || '32.60';
      }

      let restored = false;
      const restorePricing = async () => {
        if (restored) return;
        await adminPage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
        await openPricingRow(adminPage, 'USD');

        // 1. Restore published tier prices via pricingApprove if TierEditor is enabled.
        const approveBtn = tierEditor.locator('button[name="approve"]');
        if (await approveBtn.isEnabled().catch(() => false)) {
          for (const t of [1, 2, 3, 4]) {
            await tierEditor.locator(`input[name="buy-${t}"]`).fill(origTierBuys[t]);
            await tierEditor.locator(`input[name="sell-${t}"]`).fill(origTierSells[t]);
          }
          await tierEditor.locator('input[name="reason"]').fill(`${RUN_MARKER}-restore-approve`);
          const revApp = await posted(adminPage, ROUTE_PRICING_APPROVE, () => approveBtn.click());
          expect(revApp.ok, `restore pricingApprove: ${revApp.text}`).toBeTruthy();
        }

        // 2. Restore policy override state via pricingConfigure (clearOverride if originally not overridden, else save original margins).
        await openPricingRow(adminPage, 'USD');
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
        // Step 1: Configure pricing policy on USD as admin (POST /api/pricing/configure, PricingPolicy.tsx:30-34,113).
        const cfgBuy = ['0.05', '0.12', '0.18', '0.25'];
        const cfgSell = ['-0.05', '-0.12', '-0.18', '-0.25'];
        const cfgSpread = '0.20';
        const cfgReason = `${RUN_MARKER}-configure`;

        for (const t of [1, 2, 3, 4]) {
          await policyForm.locator(`input[name="buy-margin-${t}"]`).fill(cfgBuy[t - 1]);
          await policyForm.locator(`input[name="sell-margin-${t}"]`).fill(cfgSell[t - 1]);
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
        await openPricingRow(adminPage, 'USD');
        await expect(
          adminPage.locator('tr.pricing-row[data-currency="USD"] .pricing-badge-overridden'),
          'USD row shows overridden badge after pricingConfigure',
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

        // Step 2: Approve tier prices (POST /api/pricing/approve, TierEditor.tsx:36-40,87).
        // If finance holds RatePolicySave + RatesDraft, approve directly from financePage; otherwise approve on
        // adminPage and verify on financePage (and if draftState === 'PENDING_APPROVAL', finance approves on /#approvals).
        const financePerms = financePage ? await mePermissions(financePage) : [];
        const financeCanApproveBoard =
          financePage !== null &&
          financePerms.includes('RatePolicySave') &&
          financePerms.includes('RatesDraft');
        const approverPage = financeCanApproveBoard ? financePage! : adminPage;

        if (approverPage === financePage) {
          await financePage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
          await openPricingRow(financePage, 'USD');
        }

        const approverEditor = approverPage.locator('section.tier-editor');
        const baseBuyNum = Number(origTierBuys[1]) > 0 ? Number(origTierBuys[1]) : 32.1;
        const baseSellNum = Number(origTierSells[1]) > baseBuyNum ? Number(origTierSells[1]) : baseBuyNum + 0.5;
        const approvedBuys = [0, 0.01, 0.02, 0.03].map((d) => (baseBuyNum + d).toFixed(2));
        const approvedSells = [0, -0.01, -0.02, -0.03].map((d) => (baseSellNum + d).toFixed(2));

        for (const t of [1, 2, 3, 4]) {
          await approverEditor.locator(`input[name="buy-${t}"]`).fill(approvedBuys[t - 1]);
          await approverEditor.locator(`input[name="sell-${t}"]`).fill(approvedSells[t - 1]);
        }
        await approverEditor.locator('input[name="reason"]').fill(`${RUN_MARKER}-approve`);

        const appRes = await posted(approverPage, ROUTE_PRICING_APPROVE, () =>
          approverEditor.locator('button[name="approve"]').click(),
        );
        expect(appRes.ok, `POST /api/pricing/approve: ${appRes.text}`).toBeTruthy();
        const appBody = JSON.parse(appRes.body) as {
          policyVersionId: string;
          version: number;
          draftId: string;
          draftState: string;
        };
        expect(['PUBLISHED', 'PENDING_APPROVAL']).toContain(appBody.draftState);

        // If out-of-bound draft waits in A06 (PENDING_APPROVAL) and finance holds ApprovalsDecide, finance approves it.
        if (appBody.draftState === 'PENDING_APPROVAL' && financePage && financePerms.includes('ApprovalsDecide')) {
          await financePage.goto('/#approvals');
          const queueOrEmpty = financePage
            .locator('.admin-approvals-screen .approvals-queue, .admin-approvals-screen .empty')
            .first();
          await expect(queueOrEmpty).toBeVisible({ timeout: 20_000 });
          const reqCard = financePage.locator('[data-request-type="RATE_PUBLICATION"]').first();
          if (await reqCard.isVisible().catch(() => false)) {
            await posted(financePage, /^\/api\/approvals\/[^/]+\/decide$/, () =>
              reqCard.locator('button[data-outcome="APPROVED"]').click(),
            );
          }
        }

        // Cross-check on finance's /#pricing view (smoke-admin-pricing.spec.ts:21-34: finance holds ReferenceRatesRead).
        if (financePage) {
          await financePage.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
          await openPricingRow(financePage, 'USD');
          await expect(
            financePage.locator('tr.pricing-row[data-currency="USD"] .pricing-badge-overridden'),
          ).toBeVisible({ timeout: 20_000 });
        }

        await approverPage.reload();
        await openPricingRow(approverPage, 'USD');
        const afterBoard = await fetchPricingBoard(approverPage, users.branch);
        expect(afterBoard?.policyVersionId).toBe(appBody.policyVersionId);
        await shot(approverPage, 'flow-admin-pricing-rates', '3-pricing-approved');

        // Restore pricing BEFORE the test ends (and verify after reload).
        await restorePricing();
        await adminPage.reload();
        await openPricingRow(adminPage, 'USD');
        if (!wasOverridden) {
          await expect(adminPage.locator('tr.pricing-row[data-currency="USD"] .pricing-badge-overridden')).toHaveCount(
            0,
          );
        }
      } finally {
        await restorePricing().catch((e) => console.warn('[pricing] restore failed:', e));
      }
    } finally {
      await financePage?.close().catch(() => {});
      await adminPage.close().catch(() => {});
    }
  });

  test('admin saves rate-alert settings on /#rate-alert-settings, reads back after reload, and restores', async ({
    page,
  }) => {
    // web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:211 (menu.ts:13 needs RateAlertSettingsSave)
    await signIn(page, 'admin', '/#rate-alert-settings');
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

      // PUT /api/rate-alert-settings (RateAlertSettings.tsx:50,192)
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

  test('admin saves branch alert settings on /#alert-settings, reads back after reload, and restores', async ({
    page,
  }) => {
    // web/src/screens/admin/alert-settings/AlertSettings.tsx:148 (menu.ts:12 needs AlertSettingsSave)
    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
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

      // Remove any floor not in origFloors
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
      // Restore or re-add each original floor
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

      // POST /api/v3/commands/branch_alert_settings_save (AlertSettings.tsx:110-128)
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

  test('pricingConfigure and pricingApprove refuse invalid inputs in th and en and preserve typed values', async ({
    page,
  }) => {
    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}#pricing`);
    await ensurePricingBoardReady(page, users.branch);

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#pricing`);
      await openPricingRow(page, 'USD');

      const policyForm = page.locator('form.pricing-policy-form');
      const policyAlert = policyForm.locator('[role="alert"].pricing-policy-alert');
      const saveBtn = policyForm.locator('button[name="save"]');

      // Case 1: negative minSpreadPct ("-0.5") -> 400 validationFailed (currency_tier_override.rs:61-68)
      await policyForm.locator('input[name="minSpreadPct"]').fill('-0.5');
      await policyForm.locator('input[name="reason"]').fill(`${RUN_MARKER}-bad-spread-neg`);
      const negSpreadRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
      expect(negSpreadRes.ok).toBeFalsy();
      expect(negSpreadRes.text).toMatch(/^400\b/);
      await expect(policyAlert).toBeVisible();
      expect((await policyAlert.innerText()).trim()).not.toBe('validationFailed');
      await expect(policyForm.locator('input[name="minSpreadPct"]')).toHaveValue('-0.5');

      // Case 2: out-of-range minSpreadPct ("6.5", > 5) -> 400 validationFailed (currency_tier_override.rs:61-68)
      await policyForm.locator('input[name="minSpreadPct"]').fill('6.5');
      const highSpreadRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
      expect(highSpreadRes.ok).toBeFalsy();
      expect(highSpreadRes.text).toMatch(/^400\b/);
      await expect(policyAlert).toBeVisible();
      await expect(policyForm.locator('input[name="minSpreadPct"]')).toHaveValue('6.5');
      await policyForm.locator('input[name="minSpreadPct"]').fill('');

      // Case 3: too many decimals on tier buy margin ("0.12345", scale > 4) -> 400 validationFailed (rate_percent.rs:17-24)
      await policyForm.locator('input[name="buy-margin-1"]').fill('0.12345');
      const badScaleRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
      expect(badScaleRes.ok).toBeFalsy();
      expect(badScaleRes.text).toMatch(/^400\b/);
      await expect(policyAlert).toBeVisible();
      await expect(policyForm.locator('input[name="buy-margin-1"]')).toHaveValue('0.12345');

      // Case 4: out-of-range tier margin ("150.00" / >100) -> 400 validationFailed
      await policyForm.locator('input[name="buy-margin-1"]').fill('150.00');
      const hugeMarginRes = await posted(page, ROUTE_PRICING_CONFIGURE, () => saveBtn.click());
      expect(hugeMarginRes.ok).toBeFalsy();
      expect(hugeMarginRes.text).toMatch(/^400\b/);
      await expect(policyAlert).toBeVisible();
      await expect(policyForm.locator('input[name="buy-margin-1"]')).toHaveValue('150.00');

      // Case 5: TierEditor invalid tier price ("-10" on buy-1) -> 400 validationFailed (pricing_approve.rs:201-208)
      const tierEditor = page.locator('section.tier-editor');
      await tierEditor.locator('input[name="buy-1"]').fill('-10');
      await tierEditor.locator('input[name="reason"]').fill(`${RUN_MARKER}-bad-approve`);
      const badApproveRes = await posted(page, ROUTE_PRICING_APPROVE, () =>
        tierEditor.locator('button[name="approve"]').click(),
      );
      expect(badApproveRes.ok).toBeFalsy();
      expect(badApproveRes.text).toMatch(/^400\b/);
      const tierAlert = tierEditor.locator('[role="alert"].tier-editor-alert');
      await expect(tierAlert).toBeVisible();
      expect((await tierAlert.innerText()).trim()).not.toBe('validationFailed');
      await expect(tierEditor.locator('input[name="buy-1"]')).toHaveValue('-10');
    }
  });

  test('orgStartingPriceSave refuses invalid inputs in th and en and preserves typed values', async ({ page }) => {
    // web/src/screens/admin/starting-prices/StartingPrices.tsx:186-277, i18n.ts:34-44,78-88
    await signIn(page, 'admin', '/#starting-prices');

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto('/#starting-prices');
      const screen = page.locator('.xc-sp-screen');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const usdRow = screen.locator('tr.xc-sp-currency-row[data-currency="USD"]');
      if ((await usdRow.count()) === 0) {
        await screen.locator('input.xc-sp-add-code-input').fill('USD');
        await screen.locator('button.xc-sp-add-code-btn').click();
      }
      const buyInput = usdRow.locator('input.xc-sp-input[data-field="buy"]');
      const sellInput = usdRow.locator('input.xc-sp-input[data-field="sell"]');
      const reasonInput = screen.locator('input#xc-sp-reason[name="reason"]');
      const saveBtn = screen.locator('button.xc-sp-save-btn[data-action="save"]');

      // Case 1: empty reason -> .xc-sp-reason-error
      await buyInput.fill('32.10');
      await sellInput.fill('32.60');
      await reasonInput.fill('');
      await saveBtn.click();
      await expect(screen.locator('.xc-sp-reason-error')).toHaveText(
        lang === 'th' ? 'กรุณาระบุเหตุผล' : 'Reason is required',
      );
      await expect(buyInput).toHaveValue('32.10');
      await expect(sellInput).toHaveValue('32.60');

      // Case 2: negative buy price ("-5") -> .xc-sp-buy-error
      await buyInput.fill('-5');
      await sellInput.fill('32.60');
      await reasonInput.fill(`${RUN_MARKER}-neg-buy`);
      await saveBtn.click();
      await expect(usdRow.locator('.xc-sp-buy-error[data-error-for="buy"]')).toHaveText(
        lang === 'th' ? 'ต้องเป็นตัวเลขทศนิยมบวก' : 'Must be a positive decimal number',
      );
      await expect(buyInput).toHaveValue('-5');

      // Case 3: zero sell price ("0") -> .xc-sp-sell-error
      await buyInput.fill('32.10');
      await sellInput.fill('0');
      await saveBtn.click();
      await expect(usdRow.locator('.xc-sp-sell-error[data-error-for="sell"]')).toHaveText(
        lang === 'th' ? 'ต้องเป็นตัวเลขทศนิยมบวก' : 'Must be a positive decimal number',
      );
      await expect(sellInput).toHaveValue('0');

      // Case 4: buy > sell ("40.00" > "30.00") -> .xc-sp-row-error
      await buyInput.fill('40.00');
      await sellInput.fill('30.00');
      await saveBtn.click();
      await expect(usdRow.locator('.xc-sp-row-error.xc-sp-currency-error')).toHaveText(
        lang === 'th' ? 'ราคาซื้อต้องไม่เกินราคาขาย' : 'Buy price cannot be greater than sell price',
      );
      await expect(buyInput).toHaveValue('40.00');
      await expect(sellInput).toHaveValue('30.00');

      // Case 5: too many decimals ("32.123456789", >8 decimals) -> server 400 validationFailed -> .xc-sp-form-error[role="alert"]
      await buyInput.fill('32.123456789');
      await sellInput.fill('32.60');
      const res = await posted(page, ROUTE_ORG_STARTING_PRICE_SAVE, () => saveBtn.click());
      expect(res.ok).toBeFalsy();
      expect(res.text).toMatch(/^400\b/);
      await expect(screen.locator('.xc-sp-form-error[role="alert"]')).toHaveText(
        lang === 'th' ? 'ข้อมูลไม่ถูกต้อง' : 'Validation failed',
      );
      await expect(buyInput).toHaveValue('32.123456789');
    }
  });

  test('rateAlertSettingsSave refuses invalid inputs in th and en and preserves typed values', async ({ page }) => {
    // web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:142-178, i18n.ts:34-38,74-78
    await signIn(page, 'admin', '/#rate-alert-settings');

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto('/#rate-alert-settings');
      const screen = page.locator('.rate-alert-settings');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const cooldownInput = screen.locator('input#default-cooldown-hours[name="defaultCooldownHours"]');
      const maxActiveInput = screen.locator('input#max-active-per-customer[name="maxActivePerCustomer"]');
      const lifetimeInput = screen.locator('input#default-lifetime-days[name="defaultLifetimeDays"]');
      const saveBtn = screen.locator('button.rate-alert-submit-btn[type="submit"]');

      // Case 1: zero cooldown ("0") -> [data-error="cooldown"]
      await cooldownInput.fill('0');
      await maxActiveInput.fill('10');
      await lifetimeInput.fill('30');
      await saveBtn.click();
      await expect(screen.locator('[data-error="cooldown"]')).toHaveText(
        lang === 'th'
          ? 'ระยะเวลา Cooldown ต้องเป็นตัวเลขบวกที่มากกว่า 0'
          : 'Cooldown must be a positive number greater than 0',
      );
      await expect(cooldownInput).toHaveValue('0');

      // Case 2: negative cooldown ("-2") -> [data-error="cooldown"]
      await cooldownInput.fill('-2');
      await saveBtn.click();
      await expect(screen.locator('[data-error="cooldown"]')).toHaveText(
        lang === 'th'
          ? 'ระยะเวลา Cooldown ต้องเป็นตัวเลขบวกที่มากกว่า 0'
          : 'Cooldown must be a positive number greater than 0',
      );
      await expect(cooldownInput).toHaveValue('-2');

      // Case 3: non-integer maxActivePerCustomer ("2.5") -> [data-error="maxActive"]
      await cooldownInput.fill('24');
      await maxActiveInput.fill('2.5');
      await saveBtn.click();
      await expect(screen.locator('[data-error="maxActive"]')).toHaveText(
        lang === 'th'
          ? 'จำนวนการแจ้งเตือนสูงสุดต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0'
          : 'Max active alerts must be a positive integer greater than 0',
      );
      await expect(maxActiveInput).toHaveValue('2.5');

      // Case 4: negative defaultLifetimeDays ("-5") -> [data-error="lifetime"]
      await maxActiveInput.fill('10');
      await lifetimeInput.fill('-5');
      await saveBtn.click();
      await expect(screen.locator('[data-error="lifetime"]')).toHaveText(
        lang === 'th'
          ? 'อายุการแจ้งเตือนต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0 หรือเว้นว่างไว้'
          : 'Lifetime must be a positive integer greater than 0 or left blank',
      );
      await expect(lifetimeInput).toHaveValue('-5');
    }
  });

  test('branchAlertSettingsSave refuses invalid inputs in th and en and preserves typed values', async ({ page }) => {
    // web/src/screens/admin/alert-settings/AlertSettings.tsx:148, branch_alert_settings_save.rs:60-105
    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}#alert-settings`);

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#alert-settings`);
      const screen = page.locator('.alert-settings-container');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const staleInput = screen.locator('input#stale-data-minutes');
      const addCurrencyInput = screen.locator('input#add-currency');
      const addFloorBtn = screen.locator('button.add-floor-button');
      const saveBtn = screen.locator('.alert-settings-actions button[type="button"]');
      const errorAlert = screen.locator('.alert-settings-error[role="alert"]');

      // Case 1: zero staleDataMinutes ("0") -> 400 validationFailed
      await staleInput.fill('0');
      const zeroRes = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(zeroRes.ok).toBeFalsy();
      expect(zeroRes.text).toMatch(/^400\b/);
      await expect(errorAlert).toBeVisible();
      expect((await errorAlert.innerText()).trim()).not.toBe('validationFailed');
      await expect(staleInput).toHaveValue('0');

      // Case 2: negative staleDataMinutes ("-10") -> 400 validationFailed
      await staleInput.fill('-10');
      const negStaleRes = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(negStaleRes.ok).toBeFalsy();
      expect(negStaleRes.text).toMatch(/^400\b/);
      await expect(errorAlert).toBeVisible();
      await expect(staleInput).toHaveValue('-10');
      await staleInput.fill('30');

      // Ensure USD floor row is present for floor input validation cases
      if ((await screen.locator('[data-floor="USD"]').count()) === 0) {
        await addCurrencyInput.fill('USD');
        await addFloorBtn.click();
      }
      const usdFloorInput = screen.locator('[data-floor="USD"] input[type="text"]');

      // Case 3: negative floor minQuantity ("-50") -> 400 validationFailed
      await usdFloorInput.fill('-50');
      const negFloorRes = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(negFloorRes.ok).toBeFalsy();
      expect(negFloorRes.text).toMatch(/^400\b/);
      await expect(errorAlert).toBeVisible();
      await expect(usdFloorInput).toHaveValue('-50');

      // Case 4: too many decimals on floor minQuantity ("100.12345", scale > 4) -> 400 validationFailed
      await usdFloorInput.fill('100.12345');
      const scaleFloorRes = await posted(page, ROUTE_BRANCH_ALERT_SETTINGS_SAVE, () => saveBtn.click());
      expect(scaleFloorRes.ok).toBeFalsy();
      expect(scaleFloorRes.text).toMatch(/^400\b/);
      await expect(errorAlert).toBeVisible();
      await expect(usdFloorInput).toHaveValue('100.12345');
    }
  });

  // ── (c) Permission ────────────────────────────────────────────────────────

  test('teller sees none of the 4 menus (smoke-rbac-matrix.spec.ts) and direct writes are refused with 403', async ({
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

    // Direct command requests as teller must be refused with 403 (roleNotAllowed / roleNotAllowedHere).
    const configureRes = await page.request.post(
      `/api/pricing/configure?branch=${encodeURIComponent(users.branch)}`,
      {
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': crypto.randomUUID(),
        },
        data: {
          policyVersionId: crypto.randomUUID(),
          currency: 'USD',
          tiers: null,
          minSpreadPct: null,
          reason: `${RUN_MARKER}-teller-refused`,
        },
      },
    );
    expect(configureRes.status(), 'teller refused on POST /api/pricing/configure').toBe(403);

    const approveRes = await page.request.post(
      `/api/pricing/approve?branch=${encodeURIComponent(users.branch)}`,
      {
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': crypto.randomUUID(),
        },
        data: {
          policyVersionId: crypto.randomUUID(),
          rows: [
            {
              currency: 'USD',
              tiers: [
                { tierNo: 1, buy: '32.10', sell: '32.60' },
                { tierNo: 2, buy: '32.12', sell: '32.58' },
                { tierNo: 3, buy: '32.14', sell: '32.56' },
                { tierNo: 4, buy: '32.16', sell: '32.54' },
              ],
            },
          ],
          reason: `${RUN_MARKER}-teller-refused`,
        },
      },
    );
    expect(approveRes.status(), 'teller refused on POST /api/pricing/approve').toBe(403);

    const spRes = await page.request.post('/api/v3/commands/org_starting_price_save', {
      headers: {
        'x-xchange-request': '1',
        'idempotency-key': crypto.randomUUID(),
      },
      data: {
        entries: [{ currency: 'USD', buy: '32.10', sell: '32.60' }],
        reason: `${RUN_MARKER}-teller-refused`,
      },
    });
    expect(spRes.status(), 'teller refused on POST /api/v3/commands/org_starting_price_save').toBe(403);

    const rasRes = await page.request.put('/api/rate-alert-settings', {
      headers: {
        'x-xchange-request': '1',
        'idempotency-key': crypto.randomUUID(),
      },
      data: {
        defaultCooldownSeconds: 3600,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      },
    });
    expect(rasRes.status(), 'teller refused on PUT /api/rate-alert-settings').toBe(403);

    const basRes = await page.request.post(
      `/api/v3/commands/branch_alert_settings_save?branch=${encodeURIComponent(users.branch)}`,
      {
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': crypto.randomUUID(),
        },
        data: {
          staleDataMinutes: 30,
          floors: [{ currency: 'USD', minQuantity: '1000' }],
        },
      },
    );
    expect(basRes.status(), 'teller refused on POST /api/v3/commands/branch_alert_settings_save').toBe(403);
  });
});
