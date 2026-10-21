// QA-R9-2 — flow: admin campaigns and coupons (design docs/design/qa-e2e.md §9.1–§9.5 row R9-2; blueprint/marketing-campaigns.md §4.5, §7, §8).
// Written by ticket #4286 (QA-R9-2); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Commands covered (docs/design/qa-e2e.md §9.4 rows 36–42, 60):
// - campaignCreate:      POST /api/campaigns                       (web/src/screens/admin/campaigns/Campaigns.tsx:156,452)
// - campaignVersionSave: PUT  /api/campaigns/{id}/draft            (web/src/screens/admin/campaigns/Campaigns.tsx:157-163,504)
// - campaignSimulate:    POST /api/campaigns/{id}/simulate         (web/src/screens/admin/campaigns/Campaigns.tsx:164-168,530)
// - campaignSubmit:      POST /api/campaigns/{id}/submit           (web/src/screens/admin/campaigns/Campaigns.tsx:169-173,552)
// - campaignPause:       POST /api/campaigns/{id}/pause            (web/src/screens/admin/campaigns/Campaigns.tsx:174-178,568)
// - campaignResume:      POST /api/campaigns/{id}/resume           (web/src/screens/admin/campaigns/Campaigns.tsx:179-183,584)
// - campaignVersionNew:  POST /api/campaigns/{id}/versions         (web/src/screens/admin/campaigns/Campaigns.tsx:184-188,617)
// - couponIssue:         POST /api/campaigns/{id}/coupons          (web/src/screens/admin/campaigns/Campaigns.tsx:189-193,604)
//
// Selectors cited from web/src/screens/admin/:
// - campaigns/menu.ts:12-13
//   id: 'campaigns', needs: 'MarketingCampaigns'
// - campaigns/Campaigns.tsx:658-749 (M01 list view)
//   .xc-campaigns-screen (:658), .xc-campaigns-header button.xc-btn-primary (:665-671),
//   .xc-campaigns-empty button.xc-btn-primary (:696-702),
//   tr.xc-campaign-row[data-campaign-id="<id>"] (:723-727), .xc-status-badge (:731)
// - campaigns/Campaigns.tsx:755-885 (Create Campaign form)
//   .xc-campaign-create (:755), .xc-campaign-create .xc-detail-actions button.xc-btn-secondary (:759-765),
//   .xc-campaign-create .xc-detail-actions button.xc-btn-primary (:766-773),
//   .xc-campaign-create .xc-campaigns-error (:777-783),
//   input[name="name"][data-field="name"] (:788-796),
//   input[type="date"][name="startsAt"][data-field="startsAt"] (:801-809),
//   input[type="date"][name="endsAt"][data-field="endsAt"] (:814-822),
//   input[name="budgetThb"][data-field="budgetThb"] (:827-835),
//   input[name="stackGroup"][data-field="stackGroup"] (:840-848),
//   input[type="number"][name="priority"][data-field="priority"] (:853-861),
//   input[name="branches"][data-field="branches"] (:866-882)
// - campaigns/Campaigns.tsx:889-1156 (M02 detail view & coupon issue)
//   .xc-campaign-detail (:889), .xc-campaign-detail .xc-detail-header button.xc-btn-secondary (:892-902),
//   [data-testid="campaign-status"] (:903-908),
//   .xc-detail-actions button.xc-btn-primary (Save, :912-919),
//   .xc-detail-actions button.xc-btn-secondary (Simulate :921-928, Submit :930-937, Resume :950-959, New version :961-970),
//   .xc-detail-actions button.xc-btn-warning (Pause, :939-948),
//   .xc-campaign-detail .xc-campaigns-error (:974-980),
//   input[data-field="name"] (:986-994), input[data-field="budgetThb"] (:997-1005),
//   input[data-field="startsAt"] (:1008-1016), input[data-field="endsAt"] (:1019-1027),
//   input[data-field="stackGroup"] (:1030-1038), input[data-field="priority"] (:1041-1049),
//   .xc-coupon-issue-section (:1073), select[name="ruleId"] (:1078-1092),
//   input[name="count"] (:1097-1105), input[type="date"][name="expiresAt"] (:1110-1117),
//   .xc-coupon-issue-section > button.xc-btn-secondary (Issue coupons, :1121-1128),
//   .xc-issued-coupons (:1134), .xc-coupon-item (:1141), .xc-coupon-id (:1142), .xc-coupon-code (:1144)
// - campaigns/CampaignRulesEditor.tsx:56-268 (M02 benefit rules editor)
//   [data-rules-editor] (:56), [data-rules-editor] .xc-detail-header button.xc-btn-secondary (Add rule, :60-67),
//   .xc-rule-row[data-rule-row="<n>"] (:75), select[name="benefitType"] (:79-93),
//   select[name="calcMode"] (:98-113), input[name="calcValue"] (:118-126),
//   input[name="currencies"] (:131-139), select[name="shopSide"] (:144-173),
//   input[name="minThb"] (:178-186), input[name="perTransactionMaxThb"] (:191-199),
//   input[name="perCustomerUses"] (:204-212), input[name="perCustomerPeriodDays"] (:217-225),
//   input[type="checkbox"][name="requiresCoupon"] (:230-236),
//   [data-field-error="rules.calculation"] (:254), [data-field-error="rules.caps"] (:260)
// - approvals/Approvals.tsx:152-249,319-358 (executive approval of CAMPAIGN_VERSION)
//   .admin-approvals-screen (:319), [data-type-filter="CAMPAIGN_VERSION"] (:351),
//   [data-request="<id>"] (:152), button.primary (Approve, :237-244)
import { expect, test, type Browser, type Page } from '@playwright/test';
import {
  hasAccount,
  mePermissions,
  posted,
  shot,
  signIn,
  STAFF_LANGS,
  users,
  withLang,
  type Lang,
  type Role,
} from './helpers';

const MARKER_PREFIX = 'qa-r9-2';

// Raw error codes in web/src/screens/admin/campaigns/Campaigns.tsx:58-66 that must never appear unworded.
const RAW_ERROR_CODES =
  /\b(simulationRequired|staleVersion|approvalRequired|validationFailed|outOfScope|idempotencyConflict|couponInvalid|serverError|httpError|calcInvalid)\b/;

// Date in Asia/Bangkok offset by `daysAhead` days (yyyy-mm-dd), matching flow-booking.spec.ts:7-10.
function bangkokDate(daysAhead: number): string {
  const now = new Date(Date.now() + 7 * 3600_000 + daysAhead * 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

function campaignsPathFor(role: Role): string {
  return role === 'admin' ? `/?branch=${encodeURIComponent(users.branch)}#campaigns` : '/#campaigns';
}

async function resolveCampaignRole(browser: Browser): Promise<Role> {
  if (hasAccount('marketing')) {
    const page = await browser.newPage({ baseURL: users.baseUrl });
    try {
      await signIn(page, 'marketing', '/#campaigns');
      const perms = await mePermissions(page);
      if (perms.includes('MarketingCampaigns')) {
        return 'marketing';
      }
    } finally {
      await page.close().catch(() => {});
    }
  }
  return 'admin';
}

function assertWordedRefusal(text: string, lang: Lang, label: string): void {
  const trimmed = text.trim();
  expect(trimmed.length, `${label} (${lang}): refusal text is non-empty`).toBeGreaterThan(0);
  expect(trimmed, `${label} (${lang}): refusal is worded, not a raw error code`).not.toMatch(RAW_ERROR_CODES);
  if (lang === 'th') {
    expect(trimmed, `${label} (th): refusal contains Thai script`).toMatch(/[\u0E00-\u0E7F]/);
  } else {
    expect(trimmed, `${label} (en): refusal contains Latin text and no Thai script`).toMatch(/[A-Za-z]/);
    expect(trimmed, `${label} (en): refusal contains no Thai script`).not.toMatch(/[\u0E00-\u0E7F]/);
  }
}

async function openCampaignsScreen(page: Page, role: Role): Promise<void> {
  await page.goto(campaignsPathFor(role));
  // Root container .xc-campaigns-screen (Campaigns.tsx:658).
  await expect(page.locator('.xc-campaigns-screen')).toBeVisible({ timeout: 20_000 });
}

async function openCreateCampaignForm(page: Page): Promise<void> {
  const screen = page.locator('.xc-campaigns-screen');
  await expect(screen).toBeVisible({ timeout: 20_000 });
  // Header create button (Campaigns.tsx:665-671) or empty-state create button (Campaigns.tsx:696-702).
  const createBtn = screen
    .locator('.xc-campaigns-header button.xc-btn-primary, .xc-campaigns-empty button.xc-btn-primary')
    .first();
  await expect(createBtn, 'create campaign button on M01 list').toBeVisible({ timeout: 15_000 });
  await createBtn.click();
  await expect(screen.locator('.xc-campaign-create')).toBeVisible({ timeout: 15_000 });
}

async function createDraftCampaignThroughUi(
  page: Page,
  role: Role,
  name: string,
  startsAt = bangkokDate(0),
  endsAt = bangkokDate(30),
  budgetThb = '50000.00',
): Promise<string> {
  await openCampaignsScreen(page, role);
  await openCreateCampaignForm(page);

  // Create form inputs (Campaigns.tsx:788-882).
  const form = page.locator('.xc-campaign-create');
  await form.locator('input[name="name"][data-field="name"]').fill(name);
  await form.locator('input[name="startsAt"][data-field="startsAt"]').fill(startsAt);
  await form.locator('input[name="endsAt"][data-field="endsAt"]').fill(endsAt);
  await form.locator('input[name="budgetThb"][data-field="budgetThb"]').fill(budgetThb);
  await form.locator('input[name="stackGroup"][data-field="stackGroup"]').fill('DEFAULT');
  await form.locator('input[name="priority"][data-field="priority"]').fill('10');
  await form.locator('input[name="branches"][data-field="branches"]').fill(users.branch);

  // Submit create button (Campaigns.tsx:766-773) -> POST /api/campaigns (:156,452).
  const submitBtn = form.locator('.xc-detail-actions button.xc-btn-primary');
  const createRes = await posted(page, /^\/api\/campaigns$/, () => submitBtn.click());
  console.log(`[R9-2] POST /api/campaigns (${name}) -> ${createRes.text}`);
  expect(createRes.ok, `POST /api/campaigns (${name}): ${createRes.text}`).toBeTruthy();

  const parsed = JSON.parse(createRes.body) as { id?: string };
  const campaignId = String(parsed.id ?? '').trim();
  expect(campaignId, 'created campaign id').not.toBe('');

  // Campaigns.tsx:455-458 opens .xc-campaign-detail (:889) automatically after creation.
  await expect(page.locator('.xc-campaign-detail')).toBeVisible({ timeout: 20_000 });
  return campaignId;
}

async function openCampaignRowAfterReload(page: Page, role: Role, campaignId: string, expectedName: string): Promise<void> {
  await page.reload();
  await openCampaignsScreen(page, role);
  // M01 table row tr.xc-campaign-row[data-campaign-id="<id>"] (Campaigns.tsx:723-727).
  const row = page.locator(`tr.xc-campaign-row[data-campaign-id="${campaignId}"]`);
  await expect(row, `campaign row ${campaignId} visible after reload`).toBeVisible({ timeout: 20_000 });
  await expect(row).toContainText(expectedName);
  await row.click();
  await expect(page.locator('.xc-campaign-detail')).toBeVisible({ timeout: 20_000 });
}

async function findPendingCampaignApprovalId(admin: Page, campaignId: string): Promise<string> {
  const detailRes = await admin.request.get(
    `/api/v3/views/campaign?branch=${encodeURIComponent(users.branch)}&id=${encodeURIComponent(campaignId)}`,
  );
  if (detailRes.ok()) {
    const body = (await detailRes.json()) as {
      data?: {
        campaign?: {
          versions?: Array<{ id?: string; approval?: { id?: string; status?: string } | null }>;
        } | null;
      };
    };
    const version = body.data?.campaign?.versions?.[0];
    if (version?.approval?.id) {
      return version.approval.id;
    }
    if (version?.id) {
      const queueRes = await admin.request.get(
        `/api/v3/views/approval_requests?branch=${encodeURIComponent(users.branch)}&status=PENDING&type=CAMPAIGN_VERSION`,
      );
      expect(queueRes.ok(), `GET /api/v3/views/approval_requests: ${queueRes.status()}`).toBeTruthy();
      const queueBody = (await queueRes.json()) as {
        data?: { requests?: Array<{ id: string; objectId: string; status: string }> };
      };
      const match = queueBody.data?.requests?.find((r) => r.objectId === version.id && r.status === 'PENDING');
      expect(match?.id, `pending CAMPAIGN_VERSION approval request for version ${version.id}`).toBeTruthy();
      return match!.id;
    }
  }
  throw new Error(`Unable to resolve pending CAMPAIGN_VERSION approval id for campaign ${campaignId}`);
}

async function approveCampaignOnAdminScreen(admin: Page, approvalId: string): Promise<void> {
  // Approvals.tsx:319-358,152-249: filter by CAMPAIGN_VERSION (:351) and click Approve (:237-244).
  await admin.goto(`/?branch=${encodeURIComponent(users.branch)}#approvals`);
  const screen = admin.locator('.admin-approvals-screen');
  await expect(screen).toBeVisible({ timeout: 20_000 });
  const filterBtn = screen.locator('[data-type-filter="CAMPAIGN_VERSION"]');
  await expect(filterBtn).toBeVisible({ timeout: 15_000 });
  await filterBtn.click();

  const card = screen
    .locator(`[data-request="${approvalId}"]`)
    .filter({ has: admin.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) })
    .first();
  await expect(card, `pending CAMPAIGN_VERSION card ${approvalId} on /#approvals`).toBeVisible({ timeout: 20_000 });
  const decided = await posted(admin, /^\/api\/approvals\/[^/]+\/decide$/, () =>
    card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click(),
  );
  console.log(`[R9-2:admin] POST /api/approvals/${approvalId}/decide -> ${decided.text}`);
  expect(decided.ok, `admin approved CAMPAIGN_VERSION ${approvalId}: ${decided.text}`).toBeTruthy();
}

test.describe.serial('QA-R9-2 flow: admin campaigns and coupons', () => {
  // (c) Permission: teller has no MarketingCampaigns grant (web/src/screens/admin/campaigns/menu.ts:13;
  // smoke-rbac-matrix.spec.ts:55-56,120-170) and is refused campaign writes.
  test('(c) permission: teller lacks MarketingCampaigns menu and is refused POST /api/campaigns', async ({ page }) => {
    if (!hasAccount('teller')) {
      test.skip(true, 'no teller account in users.json: run scripts/reseed.sh');
    }
    await signIn(page, 'teller', '/');
    const perms = await mePermissions(page);
    expect(perms, 'teller permissions must not include MarketingCampaigns').not.toContain('MarketingCampaigns');

    // Cite smoke-rbac-matrix.spec.ts:55-56,141-170: nav.admin-nav a[href="#campaigns"] is absent and #campaigns
    // falls back to the lowest-order allowed menu without mounting .xc-campaigns-screen (Campaigns.tsx:658).
    await page.goto('/#campaigns');
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('nav.admin-nav a[href="#campaigns"]')).toHaveCount(0);
    await expect(page.locator('.xc-campaigns-screen')).toHaveCount(0);

    // Direct command call as teller is refused with 400 (roleNotAllowed) or 403 (outOfScope).
    const res = await page.request.post('/api/campaigns', {
      headers: {
        'content-type': 'application/json',
        'idempotency-key': `qa-r9-2-teller-${Date.now()}`,
        'x-xchange-request': '1',
        origin: new URL(users.baseUrl).origin,
      },
      data: {
        name: `${MARKER_PREFIX}-teller-${Date.now()}`,
        startsAt: `${bangkokDate(0)}T00:00:00+07:00`,
        endsAt: `${bangkokDate(30)}T23:59:59+07:00`,
        budgetThb: '10000.00',
        stackGroup: 'DEFAULT',
        priority: 10,
        branches: [users.branch],
      },
    });
    const body = await res.text();
    console.log(`[R9-2 teller] POST /api/campaigns -> ${res.status()} ${body.slice(0, 200)}`);
    // Contract (policy_error.rs:104-113): a role refusal is 400 roleNotAllowed(Here); only outOfScope is 403.
    const code = /"code":"(\w+)"/.exec(body)?.[1] ?? '';
    const ok = (res.status() === 400 && /^roleNotAllowed(Here)?$/.test(code)) || (res.status() === 403 && code === 'outOfScope');
    expect(ok, `teller POST /api/campaigns refused: ${res.status()} ${body}`).toBe(true);
  });

  // (b.1) Invalid input: end before start (endsAt < startsAt) on .xc-campaign-create in th and en.
  test('(b.1) invalid input: end before start on create form is refused with worded error in th and en', async ({
    browser,
  }) => {
    if (!hasAccount('marketing') && !hasAccount('admin')) {
      test.skip(true, 'no marketing or admin account in users.json: run scripts/reseed.sh');
    }
    const role = await resolveCampaignRole(browser);
    const runMarker = `${MARKER_PREFIX}-${Date.now()}`;

    for (const lang of STAFF_LANGS) {
      const page = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await withLang(page, lang);
        await signIn(page, role, campaignsPathFor(role));
        await openCampaignsScreen(page, role);
        await openCreateCampaignForm(page);

        const form = page.locator('.xc-campaign-create');
        const typedName = `${runMarker}-end-before-start-${lang}`;
        const startsAt = bangkokDate(30);
        const endsAt = bangkokDate(1);
        await form.locator('input[name="name"][data-field="name"]').fill(typedName);
        await form.locator('input[name="startsAt"][data-field="startsAt"]').fill(startsAt);
        await form.locator('input[name="endsAt"][data-field="endsAt"]').fill(endsAt);
        await form.locator('input[name="budgetThb"][data-field="budgetThb"]').fill('25000.00');

        const postRes = await posted(page, /^\/api\/campaigns$/, () =>
          form.locator('.xc-detail-actions button.xc-btn-primary').click(),
        );
        expect(postRes.ok, `POST /api/campaigns refused (${lang}): ${postRes.text}`).toBe(false);

        // Error banner .xc-campaign-create .xc-campaigns-error (Campaigns.tsx:777-783).
        const errBanner = form.locator('.xc-campaigns-error');
        await expect(errBanner, `end-before-start error banner (${lang})`).toBeVisible({ timeout: 15_000 });
        assertWordedRefusal(await errBanner.innerText(), lang, 'end-before-start');

        // Form keeps what the user typed (qa-e2e.md §9.1).
        await expect(form.locator('input[name="name"][data-field="name"]')).toHaveValue(typedName);
        await expect(form.locator('input[name="startsAt"][data-field="startsAt"]')).toHaveValue(startsAt);
        await expect(form.locator('input[name="endsAt"][data-field="endsAt"]')).toHaveValue(endsAt);
        await expect(form.locator('input[name="budgetThb"][data-field="budgetThb"]')).toHaveValue('25000.00');
        await shot(page, 'admin-campaigns', `r9-2-invalid-dates-${lang}`);
      } finally {
        await page.close().catch(() => {});
      }
    }
  });

  // (b.2) Invalid input: negative and too-many-decimal benefit calculation values in [data-rules-editor]
  // and coupon count of 0 in .xc-coupon-issue-section, verified in both th and en.
  test('(b.2) invalid inputs: negative or too-many-decimal benefit and coupon count of 0 are refused and worded in th and en', async ({
    browser,
  }) => {
    if (!hasAccount('marketing') && !hasAccount('admin')) {
      test.skip(true, 'no marketing or admin account in users.json: run scripts/reseed.sh');
    }
    const role = await resolveCampaignRole(browser);
    const runMarker = `${MARKER_PREFIX}-${Date.now()}`;

    // Create one DRAFT campaign row named with the run marker to host the detail-form validation checks.
    const setupPage = await browser.newPage({ baseURL: users.baseUrl });
    let campaignId = '';
    try {
      await signIn(setupPage, role, campaignsPathFor(role));
      campaignId = await createDraftCampaignThroughUi(setupPage, role, runMarker);
    } finally {
      await setupPage.close().catch(() => {});
    }

    for (const lang of STAFF_LANGS) {
      const page = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await withLang(page, lang);
        await signIn(page, role, campaignsPathFor(role));
        await openCampaignsScreen(page, role);

        const row = page.locator(`tr.xc-campaign-row[data-campaign-id="${campaignId}"]`);
        await expect(row, `campaign row ${campaignId} (${lang})`).toBeVisible({ timeout: 20_000 });
        await row.click();

        const detail = page.locator('.xc-campaign-detail');
        await expect(detail).toBeVisible({ timeout: 20_000 });

        // 1. Negative benefit calculation value (-0.10) in [data-rules-editor] (CampaignRulesEditor.tsx:56-268).
        const editor = detail.locator('[data-rules-editor]');
        await expect(editor).toBeVisible({ timeout: 15_000 });
        const ruleRow = editor.locator('.xc-rule-row[data-rule-row="0"]');
        if ((await ruleRow.count()) === 0) {
          await editor.locator('.xc-detail-header button.xc-btn-secondary').click();
        }
        await expect(ruleRow).toBeVisible({ timeout: 10_000 });
        await ruleRow.locator('select[name="benefitType"]').selectOption('RATE_IMPROVEMENT');
        await ruleRow.locator('select[name="calcMode"]').selectOption('delta');
        const calcInput = ruleRow.locator('input[name="calcValue"]');
        await calcInput.fill('-0.10');

        const saveBtn = detail.locator('.xc-detail-actions button.xc-btn-primary');
        await saveBtn.click();

        // Client-side rule validation renders [data-field-error="rules.calculation"] (CampaignRulesEditor.tsx:254).
        const calcErr = ruleRow.locator('[data-field-error="rules.calculation"]');
        await expect(calcErr, `negative benefit error (${lang})`).toBeVisible({ timeout: 10_000 });
        assertWordedRefusal(await calcErr.innerText(), lang, 'negative-benefit');
        await expect(calcInput, 'negative benefit input preserved').toHaveValue('-0.10');

        // 2. Too-many-decimal benefit calculation value (0.123456789, > 6 decimals) -> refused by server on Save
        // (campaign_version_save.rs:254-268, rate_improvement.rs:28 MAX_SCALE = 6).
        await calcInput.fill('0.123456789');
        const badScaleRes = await posted(
          page,
          new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/draft$`),
          () => saveBtn.click(),
          'PUT',
        );
        expect(badScaleRes.ok, `too-many-decimal benefit PUT /draft refused (${lang}): ${badScaleRes.text}`).toBe(false);
        const detailErr = detail.locator('.xc-campaigns-error');
        await expect(detailErr, `too-many-decimal benefit error banner (${lang})`).toBeVisible({ timeout: 15_000 });
        assertWordedRefusal(await detailErr.innerText(), lang, 'too-many-decimal-benefit');
        await expect(calcInput, 'too-many-decimal benefit input preserved').toHaveValue('0.123456789');

        // 3. Coupon count of 0 in .xc-coupon-issue-section (Campaigns.tsx:1073-1129,591-595).
        const couponSection = detail.locator('.xc-coupon-issue-section');
        const countInput = couponSection.locator('input[name="count"]');
        await countInput.fill('0');

        let couponPosts = 0;
        const onCouponReq = (req: { method(): string; url(): string }) => {
          if (req.method() === 'POST' && new URL(req.url()).pathname.endsWith('/coupons')) {
            couponPosts++;
          }
        };
        page.on('request', onCouponReq);
        await couponSection.locator('button.xc-btn-secondary').click();
        page.off('request', onCouponReq);

        expect(couponPosts, `count=0 sends no POST /coupons (${lang})`).toBe(0);
        await expect(detailErr, `coupon count=0 error banner (${lang})`).toBeVisible({ timeout: 10_000 });
        assertWordedRefusal(await detailErr.innerText(), lang, 'coupon-count-zero');
        await expect(countInput, 'coupon count=0 input preserved').toHaveValue('0');
        await shot(page, 'admin-campaigns', `r9-2-invalid-detail-${lang}`);
      } finally {
        await page.close().catch(() => {});
      }
    }
  });

  // (b.3) Invalid input: empty name on .xc-campaign-create in th and en.
  test('(b.3) invalid input: empty campaign name on create form is refused with worded error in th and en', async ({
    browser,
  }) => {
    if (!hasAccount('marketing') && !hasAccount('admin')) {
      test.skip(true, 'no marketing or admin account in users.json: run scripts/reseed.sh');
    }
    const role = await resolveCampaignRole(browser);

    for (const lang of STAFF_LANGS) {
      const page = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await withLang(page, lang);
        await signIn(page, role, campaignsPathFor(role));
        await openCampaignsScreen(page, role);
        await openCreateCampaignForm(page);

        const form = page.locator('.xc-campaign-create');
        const startsAt = bangkokDate(1);
        const endsAt = bangkokDate(30);
        await form.locator('input[name="name"][data-field="name"]').fill('');
        await form.locator('input[name="startsAt"][data-field="startsAt"]').fill(startsAt);
        await form.locator('input[name="endsAt"][data-field="endsAt"]').fill(endsAt);
        await form.locator('input[name="budgetThb"][data-field="budgetThb"]').fill('15000.00');

        const submitBtn = form.locator('.xc-detail-actions button.xc-btn-primary');
        if (!(await submitBtn.isDisabled())) {
          await submitBtn.click();
          const errBanner = form.locator('.xc-campaigns-error');
          await expect(errBanner, `empty-name refusal banner (${lang})`).toBeVisible({ timeout: 15_000 });
          assertWordedRefusal(await errBanner.innerText(), lang, 'empty-name');
        }
        // Form keeps what was typed.
        await expect(form.locator('input[name="name"][data-field="name"]')).toHaveValue('');
        await expect(form.locator('input[name="budgetThb"][data-field="budgetThb"]')).toHaveValue('15000.00');
      } finally {
        await page.close().catch(() => {});
      }
    }
  });

  // (a) Happy path: create a campaign named with the run marker (qa-r9-2-<Date.now()>), take it through
  // draft (campaignVersionSave with a coupon-eligible benefit rule) -> simulate (campaignSimulate) ->
  // submit (campaignSubmit) -> executive approval -> pause (campaignPause) -> resume (campaignResume) ->
  // issue one coupon (couponIssue) -> new draft version (campaignVersionNew), reading back changes after page reloads.
  test('(a) happy path: create campaign -> save draft -> simulate -> submit -> pause -> resume -> issue coupon -> new version', async ({
    browser,
  }) => {
    if (!hasAccount('marketing') || !hasAccount('admin')) {
      test.skip(true, 'requires marketing and admin accounts in users.json: run scripts/reseed.sh');
    }

    const marketing = await browser.newPage({ baseURL: users.baseUrl });
    const admin = await browser.newPage({ baseURL: users.baseUrl });
    const runMarker = `${MARKER_PREFIX}-${Date.now()}`;
    const startsDate = bangkokDate(0);
    const endsDate = bangkokDate(30);
    let campaignId = '';
    let campaignApproved = false;

    try {
      await signIn(marketing, 'marketing', '/#campaigns');
      const mktPerms = await mePermissions(marketing);
      if (!mktPerms.includes('MarketingCampaigns')) {
        test.skip(true, 'marketing account lacks MarketingCampaigns permission');
      }
      await signIn(admin, 'admin', `/?branch=${encodeURIComponent(users.branch)}#approvals`);

      // 1. campaignCreate (POST /api/campaigns, Campaigns.tsx:452): create campaign named with runMarker.
      campaignId = await createDraftCampaignThroughUi(marketing, 'marketing', runMarker, startsDate, endsDate, '50000.00');
      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);

      const detail = marketing.locator('.xc-campaign-detail');
      const statusBadge = detail.getByTestId('campaign-status');
      await expect(statusBadge).toHaveClass(/xc-status-draft/);

      // 2. campaignVersionSave (PUT /api/campaigns/{id}/draft, Campaigns.tsx:504): update budgetThb and add
      // a coupon-eligible RATE_IMPROVEMENT rule in [data-rules-editor] (CampaignRulesEditor.tsx:56-268).
      await detail.locator('input[data-field="budgetThb"]').fill('60000.00');
      const editor = detail.locator('[data-rules-editor]');
      await editor.locator('.xc-detail-header button.xc-btn-secondary').click();
      const ruleRow = editor.locator('.xc-rule-row[data-rule-row="0"]');
      await expect(ruleRow).toBeVisible({ timeout: 10_000 });
      await ruleRow.locator('select[name="benefitType"]').selectOption('RATE_IMPROVEMENT');
      await ruleRow.locator('select[name="calcMode"]').selectOption('delta');
      await ruleRow.locator('input[name="calcValue"]').fill('0.05');
      await ruleRow.locator('input[name="currencies"]').fill(users.currency ?? 'USD');
      await ruleRow.locator('select[name="shopSide"]').selectOption('SELL');
      await ruleRow.locator('input[name="minThb"]').fill('1000');
      await ruleRow.locator('input[name="perTransactionMaxThb"]').fill('500');
      await ruleRow.locator('input[name="perCustomerUses"]').fill('3');
      await ruleRow.locator('input[type="checkbox"][name="requiresCoupon"]').check();

      const saveBtn = detail.locator('.xc-detail-actions button.xc-btn-primary');
      const saveRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/draft$`),
        () => saveBtn.click(),
        'PUT',
      );
      console.log(`[R9-2] PUT /api/campaigns/${campaignId}/draft -> ${saveRes.text}`);
      expect(saveRes.ok, `PUT /api/campaigns/${campaignId}/draft: ${saveRes.text}`).toBeTruthy();

      // Reload and read back persisted draft edits & rule.
      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(detail.locator('input[data-field="budgetThb"]')).toHaveValue('60000.00');
      await expect(editor.locator('.xc-rule-row[data-rule-row="0"] input[name="calcValue"]')).toHaveValue('0.05');
      await expect(editor.locator('.xc-rule-row[data-rule-row="0"] input[name="requiresCoupon"]')).toBeChecked();
      await shot(marketing, 'admin-campaigns', 'r9-2-1-draft-saved');

      // 3. campaignSimulate (POST /api/campaigns/{id}/simulate, Campaigns.tsx:530,921-928).
      const submitBtn = detail.getByRole('button', { name: /^(ส่งอนุมัติ|Submit)$/ });
      await expect(submitBtn, 'Submit disabled before simulation (Campaigns.tsx:389-391)').toBeDisabled();

      const simulateBtn = detail.getByRole('button', { name: /^(จำลองผล|Simulate)$/ });
      const simRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/simulate$`),
        () => simulateBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/simulate -> ${simRes.text}`);
      expect(simRes.ok, `POST /api/campaigns/${campaignId}/simulate: ${simRes.text}`).toBeTruthy();

      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-simulated/);
      await expect(submitBtn, 'Submit enabled after fresh simulation').toBeEnabled();

      // 4. campaignSubmit (POST /api/campaigns/{id}/submit, Campaigns.tsx:552,930-937).
      const subRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/submit$`),
        () => submitBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/submit -> ${subRes.text}`);
      expect(subRes.ok, `POST /api/campaigns/${campaignId}/submit: ${subRes.text}`).toBeTruthy();

      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-pending_approval/);

      // Approve CAMPAIGN_VERSION as admin on /#approvals so the campaign reaches ACTIVE (campaigns.rs:226-243).
      const approvalId = await findPendingCampaignApprovalId(admin, campaignId);
      await approveCampaignOnAdminScreen(admin, approvalId);
      campaignApproved = true;

      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-active/);

      // 5. campaignPause (POST /api/campaigns/{id}/pause, Campaigns.tsx:568,939-948).
      const pauseBtn = detail.locator('.xc-detail-actions button.xc-btn-warning');
      await expect(pauseBtn).toBeVisible({ timeout: 15_000 });
      const pauseRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/pause$`),
        () => pauseBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/pause -> ${pauseRes.text}`);
      expect(pauseRes.ok, `POST /api/campaigns/${campaignId}/pause: ${pauseRes.text}`).toBeTruthy();

      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-paused/);

      // 6. campaignResume (POST /api/campaigns/{id}/resume, Campaigns.tsx:584,950-959).
      const resumeBtn = detail.getByRole('button', { name: /^(ทำงานต่อ|Resume)$/ });
      await expect(resumeBtn).toBeVisible({ timeout: 15_000 });
      const resumeRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/resume$`),
        () => resumeBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/resume -> ${resumeRes.text}`);
      expect(resumeRes.ok, `POST /api/campaigns/${campaignId}/resume: ${resumeRes.text}`).toBeTruthy();

      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-active/);

      // 7. couponIssue (POST /api/campaigns/{id}/coupons, Campaigns.tsx:604,1073-1153): issue 1 coupon.
      const couponSection = detail.locator('.xc-coupon-issue-section');
      await couponSection.locator('input[name="count"]').fill('1');
      await couponSection.locator('input[name="expiresAt"]').fill(endsDate);
      const issueBtn = couponSection.locator('button.xc-btn-secondary');
      const issueRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/coupons$`),
        () => issueBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/coupons -> ${issueRes.text}`);
      expect(issueRes.ok, `POST /api/campaigns/${campaignId}/coupons: ${issueRes.text}`).toBeTruthy();

      const issuedItem = detail.locator('.xc-issued-coupons .xc-coupon-item');
      await expect(issuedItem).toHaveCount(1, { timeout: 15_000 });
      await expect(issuedItem.locator('.xc-coupon-id')).not.toHaveText(/^\s*$/);
      await expect(issuedItem.locator('.xc-coupon-code')).not.toHaveText(/^\s*$/);
      await shot(marketing, 'admin-campaigns', 'r9-2-2-coupon-issued');

      // 8. campaignVersionNew (POST /api/campaigns/{id}/versions, Campaigns.tsx:617,961-970).
      const newVerBtn = detail.getByRole('button', { name: /^(สร้างเวอร์ชันใหม่|New version)$/ });
      await expect(newVerBtn).toBeVisible({ timeout: 15_000 });
      const newVerRes = await posted(
        marketing,
        new RegExp(`^/api/campaigns/${encodeURIComponent(campaignId)}/versions$`),
        () => newVerBtn.click(),
      );
      console.log(`[R9-2] POST /api/campaigns/${campaignId}/versions -> ${newVerRes.text}`);
      expect(newVerRes.ok, `POST /api/campaigns/${campaignId}/versions: ${newVerRes.text}`).toBeTruthy();

      // Reload and verify the new version (v2) opens in DRAFT on the campaign detail view.
      await openCampaignRowAfterReload(marketing, 'marketing', campaignId, runMarker);
      await expect(statusBadge).toHaveClass(/xc-status-draft/);
    } finally {
      // Leave the approved version paused so the test campaign never alters live branch pricing (E-4).
      if (campaignApproved && campaignId) {
        await marketing.request
          .post(`/api/campaigns/${encodeURIComponent(campaignId)}/pause`, {
            headers: {
              'content-type': 'application/json',
              'idempotency-key': `qa-r9-2-cleanup-pause-${Date.now()}`,
              'x-xchange-request': '1',
              origin: new URL(users.baseUrl).origin,
            },
            data: { campaignId },
          })
          .catch(() => {});
      }
      await marketing.close().catch(() => {});
      await admin.close().catch(() => {});
    }
  });
});
