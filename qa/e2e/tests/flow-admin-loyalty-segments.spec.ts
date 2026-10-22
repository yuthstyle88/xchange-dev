// QA-R9-3 — flow: admin loyalty rules and member segments
// (design docs/design/qa-e2e.md §9.1–§9.5 ticket R9-3; ticket #4287).
// Written by ticket #4287 (QA-R9-3); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/:
// - screens/admin/loyalty/menu.ts:9-24
//   id: 'loyalty' (:10), needs: 'LoyaltyRulesManage' (:13)
// - screens/admin/loyalty/Loyalty.tsx:66-75,152-355 (smoke-admin-policy.spec.ts:94-107)
//   [role="alert"] (:154),
//   tr[data-version-id][data-kind="POINT"][data-version][data-status] (:184-189),
//   button[data-action="submit"] (:201-207, POST /api/v3/commands/loyalty_rule_submit :71-75,143),
//   section[data-form="point-draft"] (:217),
//   select[name="basis"] (:222-231), input[name="pointsRatio"] (:236-242),
//   select[name="rounding"] (:247-255), input[name="minimumBasisAmount"] (:260-266),
//   input[name="expiryMonths"] (:270-277), input[name="validFrom"] (:282-288),
//   input[name="validTo"] (:293-299),
//   button[data-action="save-draft"] (:302-308, POST /api/v3/commands/loyalty_rule_draft :66-70,133),
//   tr[data-version-id][data-kind="TIER"][data-version][data-status] (:327-332)
// - screens/admin/loyalty/i18n.ts:38-41,80-83
//   status.draft (:38,80), status.pending_approval (:39,81)
// - shared/i18n/th.ts:24-26,41 & shared/i18n/en.ts:26-28,43
//   error.loyaltyUnavailable, error.invalidRequest, error.roleNotAllowed
// - screens/admin/members/menu.ts:8-23
//   id: 'members' (:9), needs: 'MarketingMembers' (:12)
// - screens/admin/members/Members.tsx:70-75,156-332 (smoke-admin-policy.spec.ts:79-92)
//   .xc-campaigns-screen (:156), .xc-campaigns-error[role="alert"] (:159),
//   #segment-name / input[name="name"][data-field="name"] (:173-181),
//   #segment-recency-days / input[name="recency_days"][data-field="recency_days"] (:186-194),
//   #segment-frequency / input[name="frequency"][data-field="frequency"] (:199-207),
//   #segment-value-thb / input[name="value_thb"][data-field="value_thb"] (:212-220),
//   #segment-tier / input[name="tier"][data-field="tier"] (:225-233),
//   button[data-action="define"] (:239-245, POST /api/segments :70,113),
//   button[data-action="snapshot"] (:247-254, POST /api/segments/{id}/snapshots :71-75,142),
//   .xc-campaign-detail snapshot and preview sections (:259-331, GET /api/v3/views/segment_preview :125-130),
//   .xc-campaigns-table (:302)
// - screens/admin/members/i18n.ts:33-35,69-71
//   error.validationFailed (:33,69), error.outOfScope (:34,70), error.roleNotAllowed (:35,71)
// - screens/admin/AdminShell.tsx:279-301 (smoke-rbac-matrix.spec.ts:61-64,141-169)
//   nav.admin-nav a[href="#loyalty"], nav.admin-nav a[href="#members"]
import { expect, test } from '@playwright/test';
import { hasAccount, mePermissions, posted, shot, signIn, STAFF_LANGS, users, withLang } from './helpers';

// Validation refusal text on /#members (web/src/screens/admin/members/i18n.ts:33,69).
const MEMBERS_VALIDATION_ERR: Record<'th' | 'en', string> = {
  th: 'กรุณากรอกข้อมูลให้ถูกต้องครบถ้วน',
  en: 'Please fill in all required fields correctly',
};

// Shared invalidRequest refusal text (web/src/shared/i18n/th.ts:24, en.ts:26).
const INVALID_REQUEST_ERR: Record<'th' | 'en', string> = {
  th: 'คำขอไม่ถูกต้อง',
  en: 'Invalid request',
};

// Loyalty unavailable refusal text (web/src/shared/i18n/th.ts:41, en.ts:43).
const LOYALTY_UNAVAILABLE_ERR: Record<'th' | 'en', string> = {
  th: 'ระบบคะแนนสะสมขัดข้องชั่วคราว กรุณาลองใหม่อีกครั้ง',
  en: 'The loyalty service is temporarily unavailable. Please try again.',
};

// Date in Asia/Bangkok offset by `daysAhead` days (yyyy-mm-dd), matching flow-booking.spec.ts:7-10.
function bangkokDate(daysAhead: number): string {
  const now = new Date(Date.now() + 7 * 3600_000 + daysAhead * 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

test.describe.serial('QA-R9-3 admin loyalty rules and member segments', () => {
  test('(a) happy path: /#members segmentDefine and segmentSnapshot persist across reload, and /#loyalty drafts and submits a rule version', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    const marker = `qa-r9-3-${Date.now()}`;

    await signIn(page, 'admin', '/#members');
    const perms = await mePermissions(page);

    // ── Part 1: /#members segmentDefine + segmentSnapshot + reload read-back ──
    test.skip(!perms.includes('MarketingMembers'), 'signed-in admin lacks MarketingMembers');
    await page.goto('/#members');
    const membersScreen = page.locator('.xc-campaigns-screen');
    const nameInput = page.locator('#segment-name');
    await expect(nameInput, '/#members key element #segment-name (Members.tsx:174)').toBeVisible({ timeout: 20_000 });

    const recencyInput = page.locator('#segment-recency-days');
    const freqInput = page.locator('#segment-frequency');
    const valueInput = page.locator('#segment-value-thb');
    const defineBtn = membersScreen.locator('button[data-action="define"]');
    const snapshotBtn = membersScreen.locator('button[data-action="snapshot"]');

    // Before a segment definition is saved, snapshot is disabled (Members.tsx:250).
    await expect(snapshotBtn, 'snapshot disabled before segmentDefine').toBeDisabled();

    await nameInput.fill(marker);
    await recencyInput.fill('30');
    await freqInput.fill('1');
    await valueInput.fill('100');

    const defineRes = await posted(page, /^\/api\/segments$/, () => defineBtn.click());
    console.log(`[R9-3] POST /api/segments ${defineRes.text}`);
    expect(defineRes.ok, `segmentDefine accepted: ${defineRes.text}`).toBeTruthy();
    const defined = JSON.parse(defineRes.body) as { id: string; version: number };
    expect(defined.id, 'segmentDefine returns definition id').toBeTruthy();
    expect(defined.version, `first definition of ${marker} has version 1`).toBe(1);

    await expect(snapshotBtn, 'snapshot enabled after segmentDefine').toBeEnabled({ timeout: 10_000 });

    const previewLoaded = page.waitForResponse(
      (r) => r.request().method() === 'GET' && r.url().includes('/api/v3/views/segment_preview'),
      { timeout: 15_000 },
    );
    const snapRes = await posted(
      page,
      new RegExp(`^/api/segments/${encodeURIComponent(defined.id)}/snapshots$`),
      () => snapshotBtn.click(),
    );
    console.log(`[R9-3] POST /api/segments/${defined.id}/snapshots ${snapRes.text}`);
    expect(snapRes.ok, `segmentSnapshot accepted: ${snapRes.text}`).toBeTruthy();
    const snapped = JSON.parse(snapRes.body) as { id: string; memberCount: number };
    expect(snapped.id, 'segmentSnapshot returns snapshot id').toBeTruthy();
    expect(typeof snapped.memberCount).toBe('number');

    const previewResp = await previewLoaded;
    expect(previewResp.ok(), 'GET /api/v3/views/segment_preview 200').toBeTruthy();
    await expect(
      membersScreen.locator('.xc-campaigns-table'),
      'segment preview table visible after snapshot (Members.tsx:302)',
    ).toBeVisible({ timeout: 15_000 });
    await shot(page, 'admin-loyalty-segments', 'r9-3-members-happy');

    // Reload /#members and read back the persisted snapshot and append-only version increment (segment_define.rs:69-76).
    await page.reload();
    await expect(nameInput, '#segment-name visible after reload').toBeVisible({ timeout: 20_000 });

    const readBackView = await page.request.get(
      `/api/v3/views/segment_preview?id=${encodeURIComponent(snapped.id)}`,
    );
    expect(readBackView.ok(), 'persisted snapshot readable after reload').toBeTruthy();
    const readBackJson = (await readBackView.json()) as {
      data?: { snapshotId?: string; memberCount?: number };
    };
    expect(readBackJson.data?.snapshotId, 'snapshotId matches after reload').toBe(snapped.id);
    expect(readBackJson.data?.memberCount, 'memberCount matches after reload').toBe(snapped.memberCount);

    // Append-only check on the reloaded screen: defining the same marked name increments version to 2.
    await nameInput.fill(marker);
    await recencyInput.fill('30');
    const defineV2Res = await posted(page, /^\/api\/segments$/, () => defineBtn.click());
    expect(defineV2Res.ok, `second segmentDefine for ${marker} accepted: ${defineV2Res.text}`).toBeTruthy();
    const definedV2 = JSON.parse(defineV2Res.body) as { id: string; version: number };
    expect(definedV2.version, `append-only segment ${marker} increments to version 2 after reload`).toBe(2);

    // ── Part 2: /#loyalty loyaltyRuleDraft + loyaltyRuleSubmit + reload read-back ──
    if (!perms.includes('LoyaltyRulesManage')) {
      console.log('[R9-3] signed-in admin lacks LoyaltyRulesManage; skipping /#loyalty part');
      return;
    }

    await page.goto('about:blank');
    await page.goto('/#loyalty');
    const draftSection = page.locator('section[data-form="point-draft"]');
    await expect(
      draftSection,
      '/#loyalty key element section[data-form="point-draft"] (Loyalty.tsx:217)',
    ).toBeVisible({ timeout: 20_000 });

    const validFromDate = bangkokDate(1);
    await draftSection.locator('select[name="basis"]').selectOption('SETTLE_THB');
    await draftSection.locator('input[name="pointsRatio"]').fill('1');
    await draftSection.locator('select[name="rounding"]').selectOption('FLOOR');
    await draftSection.locator('input[name="minimumBasisAmount"]').fill('100');
    await draftSection.locator('input[name="expiryMonths"]').fill('12');
    await draftSection.locator('input[name="validFrom"]').fill(validFromDate);

    const draftRes = await posted(page, /^\/api\/v3\/commands\/loyalty_rule_draft$/, () =>
      draftSection.locator('button[data-action="save-draft"]').click(),
    );
    console.log(`[R9-3] POST /api/v3/commands/loyalty_rule_draft ${draftRes.text}`);

    if (!draftRes.ok && draftRes.body.includes('loyaltyUnavailable')) {
      // On hosts where the external Loyalty service is not configured (qa-e2e.md §6, helpers.ts:63-69),
      // verify the screen renders the worded loyaltyUnavailable alert and exercise loyaltyRuleSubmit as well.
      const alert = page.locator('[role="alert"]').first();
      await expect(alert, 'loyaltyUnavailable alert visible (Loyalty.tsx:154)').toBeVisible({ timeout: 10_000 });
      await expect(alert).not.toContainText('loyaltyUnavailable');
      const submitProbe = await page.request.post('/api/v3/commands/loyalty_rule_submit', {
        // v3 commands apply the same-origin rule after the policy gate (command.rs:256, extractors/origin.rs).
        headers: {
          'Idempotency-Key': `${marker}-submit`,
          'x-xchange-request': '1',
          origin: new URL(users.baseUrl).origin,
        },
        data: { versionId: '00000000-0000-0000-0000-000000000001' },
      });
      console.log(`[R9-3] POST /api/v3/commands/loyalty_rule_submit ${submitProbe.status()}`);
      expect(submitProbe.status(), 'loyaltyRuleSubmit also answers 503 when loyalty is unconfigured').toBe(503);
      await shot(page, 'admin-loyalty-segments', 'r9-3-loyalty-unavailable');
      return;
    }

    expect(draftRes.ok, `loyaltyRuleDraft accepted: ${draftRes.text}`).toBeTruthy();
    const drafted = JSON.parse(draftRes.body) as { versionId: string; status: string };
    expect(drafted.versionId, 'loyaltyRuleDraft returns versionId').toBeTruthy();
    expect(drafted.status, 'new loyalty rule version starts as DRAFT').toBe('DRAFT');

    // Reload /#loyalty and read back the DRAFT row (Loyalty.tsx:184-209).
    await page.reload();
    await expect(draftSection, 'point-draft visible after reload').toBeVisible({ timeout: 20_000 });
    const versionRow = page.locator(`tr[data-version-id="${drafted.versionId}"][data-kind="POINT"]`);
    await expect(versionRow, 'created DRAFT row visible after reload').toBeVisible({ timeout: 15_000 });
    await expect(versionRow).toHaveAttribute('data-status', 'DRAFT');

    // Submit the DRAFT row via button[data-action="submit"] (Loyalty.tsx:201-207).
    const submitBtn = versionRow.locator('button[data-action="submit"]');
    await expect(submitBtn, 'DRAFT row offers [data-action="submit"]').toBeVisible({ timeout: 10_000 });
    const submitRes = await posted(page, /^\/api\/v3\/commands\/loyalty_rule_submit$/, () => submitBtn.click());
    console.log(`[R9-3] POST /api/v3/commands/loyalty_rule_submit ${submitRes.text}`);
    expect(submitRes.ok, `loyaltyRuleSubmit accepted: ${submitRes.text}`).toBeTruthy();

    // Reload and verify status moved to PENDING_APPROVAL and submit button is gone.
    await page.reload();
    await expect(versionRow, 'submitted row visible after reload').toBeVisible({ timeout: 20_000 });
    await expect(versionRow).toHaveAttribute('data-status', 'PENDING_APPROVAL');
    await expect(versionRow.locator('button[data-action="submit"]')).toHaveCount(0);
    await shot(page, 'admin-loyalty-segments', 'r9-3-loyalty-happy');
  });

  test('(b) invalid inputs: 5 cases across /#members and /#loyalty refused with worded messages in th and en, keeping typed values', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    const marker = `qa-r9-3-invalid-${Date.now()}`;

    await signIn(page, 'admin', '/#members');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('MarketingMembers'), 'signed-in admin lacks MarketingMembers');

    for (const lang of STAFF_LANGS) {
      const langKey = lang as 'th' | 'en';
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto('/#members');

      const membersScreen = page.locator('.xc-campaigns-screen');
      const nameInput = page.locator('#segment-name');
      const recencyInput = page.locator('#segment-recency-days');
      const freqInput = page.locator('#segment-frequency');
      const valueInput = page.locator('#segment-value-thb');
      const tierInput = page.locator('#segment-tier');
      const defineBtn = membersScreen.locator('button[data-action="define"]');
      const snapshotBtn = membersScreen.locator('button[data-action="snapshot"]');
      const alert = membersScreen.locator('.xc-campaigns-error[role="alert"]');

      await expect(nameInput, `/#members (${lang}) #segment-name visible`).toBeVisible({ timeout: 20_000 });

      // Case 1: Empty segment name (segment_define.rs:60-67 -> 400 validationFailed; Members.tsx:35,45-48).
      await nameInput.fill('');
      await recencyInput.fill('30');
      await freqInput.fill('');
      await valueInput.fill('');
      await tierInput.fill('');
      const emptyNameRes = await posted(page, /^\/api\/segments$/, () => defineBtn.click());
      expect(emptyNameRes.ok, `case 1 empty name refused (${lang})`).toBe(false);
      await expect(alert, `case 1 worded refusal (${lang})`).toHaveText(MEMBERS_VALIDATION_ERR[langKey]);
      await expect(nameInput, 'case 1 keeps empty name').toHaveValue('');
      await expect(recencyInput, 'case 1 keeps recency_days 30').toHaveValue('30');
      await expect(snapshotBtn, 'snapshot stays disabled without a valid segment definition').toBeDisabled();

      // Case 2: Negative rule value (recency_days = -5; Members.tsx:86-90 client-side integer guard).
      await nameInput.fill(marker);
      await recencyInput.fill('-5');
      await defineBtn.click();
      await expect(alert, `case 2 negative recency_days refused (${lang})`).toHaveText(
        MEMBERS_VALIDATION_ERR[langKey],
      );
      await expect(nameInput, 'case 2 keeps segment name').toHaveValue(marker);
      await expect(recencyInput, 'case 2 keeps -5').toHaveValue('-5');

      // Case 3: Decimal / non-integer frequency in segment rule (frequency = 2.5; Members.tsx:94-98).
      await recencyInput.fill('30');
      await freqInput.fill('2.5');
      await defineBtn.click();
      await expect(alert, `case 3 decimal frequency refused (${lang})`).toHaveText(MEMBERS_VALIDATION_ERR[langKey]);
      await expect(freqInput, 'case 3 keeps 2.5').toHaveValue('2.5');

      // Case 4: Invalid decimal value_thb with too many decimal points ('10.12.34'; SegmentRule Decimal deserialization -> 400 invalidRequest).
      await freqInput.fill('1');
      await valueInput.fill('10.12.34');
      const badDecimalRes = await posted(page, /^\/api\/segments$/, () => defineBtn.click());
      expect(badDecimalRes.ok, `case 4 invalid value_thb refused (${lang})`).toBe(false);
      await expect(alert, `case 4 worded refusal (${lang})`).toHaveText(INVALID_REQUEST_ERR[langKey]);
      await expect(valueInput, 'case 4 keeps 10.12.34').toHaveValue('10.12.34');

      // Case 5: Empty segment definition on /#members (blank name + all rule fields cleared -> refused, snapshot stays disabled)
      // and invalid negative pointsRatio on /#loyalty (Loyalty.tsx:217-309).
      await nameInput.fill('   ');
      await recencyInput.fill('');
      await freqInput.fill('');
      await valueInput.fill('');
      await tierInput.fill('');
      const emptyDefRes = await posted(page, /^\/api\/segments$/, () => defineBtn.click());
      expect(emptyDefRes.ok, `case 5a empty segment definition refused (${lang})`).toBe(false);
      await expect(alert, `case 5a worded refusal (${lang})`).toHaveText(MEMBERS_VALIDATION_ERR[langKey]);
      await expect(snapshotBtn, 'case 5a snapshot button remains disabled').toBeDisabled();

      if (perms.includes('LoyaltyRulesManage')) {
        await page.goto('about:blank');
        await page.goto('/#loyalty');
        const draftSection = page.locator('section[data-form="point-draft"]');
        await expect(draftSection, `/#loyalty (${lang}) point-draft visible`).toBeVisible({ timeout: 20_000 });

        const ratioInput = draftSection.locator('input[name="pointsRatio"]');
        const minInput = draftSection.locator('input[name="minimumBasisAmount"]');
        await ratioInput.fill('-0.12345');
        await minInput.fill('-50');
        const loyaltyBadRes = await posted(page, /^\/api\/v3\/commands\/loyalty_rule_draft$/, () =>
          draftSection.locator('button[data-action="save-draft"]').click(),
        );
        expect(loyaltyBadRes.ok, `case 5b invalid loyalty rule draft refused (${lang})`).toBe(false);
        const loyaltyAlert = page.locator('[role="alert"]').first();
        await expect(loyaltyAlert, `case 5b loyalty alert visible (${lang})`).toBeVisible({ timeout: 10_000 });
        if (loyaltyBadRes.body.includes('loyaltyUnavailable')) {
          await expect(loyaltyAlert, `case 5b loyaltyUnavailable worded (${lang})`).toHaveText(
            LOYALTY_UNAVAILABLE_ERR[langKey],
          );
        } else {
          // Worded in the page language: Thai script in th, no Thai script in en (§9.1 b).
          await expect(loyaltyAlert, `case 5b refusal is worded, not raw code (${lang})`).not.toContainText(
            'validationFailed',
          );
          if (lang === 'th') {
            await expect(loyaltyAlert, 'case 5b refusal worded in Thai').toHaveText(/[\u0E00-\u0E7F]/);
          } else {
            await expect(loyaltyAlert, 'case 5b refusal worded in English').toHaveText(/^[^\u0E00-\u0E7F]*[A-Za-z][^\u0E00-\u0E7F]*$/);
          }
        }
        await expect(ratioInput, 'case 5b keeps typed pointsRatio').toHaveValue('-0.12345');
        await expect(minInput, 'case 5b keeps typed minimumBasisAmount').toHaveValue('-50');
      }

      await shot(page, 'admin-loyalty-segments', `r9-3-invalid-${lang}`);
    }
  });

  test('(c) permission: teller lacks LoyaltyRulesManage and MarketingMembers — menus not offered and command routes refuse with 400', async ({
    page,
  }) => {
    test.skip(!hasAccount('teller'), 'no teller account in users.json: run scripts/reseed.sh');
    await signIn(page, 'teller', '/#loyalty');

    // Menu gates (loyalty/menu.ts:13 needs LoyaltyRulesManage, members/menu.ts:12 needs MarketingMembers;
    // smoke-rbac-matrix.spec.ts:61-64,141-169; AdminShell.tsx:279-301).
    await expect(page.locator('h2.title'), 'lowest-order allowed menu heading visible for teller').toBeVisible({
      timeout: 20_000,
    });
    await expect(page.locator('nav.admin-nav a[href="#loyalty"]')).toHaveCount(0);
    await expect(page.locator('nav.admin-nav a[href="#members"]')).toHaveCount(0);
    await expect(page.locator('section[data-form="point-draft"]')).toHaveCount(0);

    await page.goto('/#members');
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('#segment-name')).toHaveCount(0);

    // Direct command calls as teller are refused at the policy gate with 400 (PolicyError::status, #4512).
    const dummyId = '00000000-0000-0000-0000-000000000001';
    const marker = `qa-r9-3-teller-${Date.now()}`;
    const checks: Array<{ name: string; url: string; data: Record<string, unknown> }> = [
      {
        name: 'loyaltyRuleDraft',
        url: '/api/v3/commands/loyalty_rule_draft',
        data: {
          kind: 'POINT',
          fields: {
            version: 1,
            basis: 'SETTLE_THB',
            pointsRatio: '1',
            rounding: 'FLOOR',
            minimumBasisAmount: '100',
            expiryMonths: 12,
            validFrom: '2026-11-01T00:00:00+07:00',
            validTo: null,
          },
        },
      },
      {
        name: 'loyaltyRuleSubmit',
        url: '/api/v3/commands/loyalty_rule_submit',
        data: { versionId: dummyId },
      },
      {
        name: 'segmentDefine',
        url: '/api/segments',
        data: { name: marker, rule: { recency_days: 30 } },
      },
      {
        name: 'segmentSnapshot',
        url: `/api/segments/${dummyId}/snapshots`,
        data: { id: dummyId },
      },
    ];

    for (const c of checks) {
      const key = `${marker}-${c.name}`;
      const res = await page.request.post(c.url, {
        headers: { 'Idempotency-Key': key },
        data: c.data,
      });
      const body = await res.text();
      console.log(`[R9-3 teller] POST ${c.url} -> ${res.status()} ${body.slice(0, 200)}`);
      expect(res.status(), `${c.name} refused for teller: ${body}`).toBe(400);
      const code = /"code":"(\w+)"/.exec(body)?.[1] ?? '';
      expect(['roleNotAllowed', 'roleNotAllowedHere'], `${c.name} refusal code: ${body}`).toContain(code);
    }
  });
});
