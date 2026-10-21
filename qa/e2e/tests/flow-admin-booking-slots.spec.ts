// QA-R9-4 — flow: admin booking policy, slot rules, slot open/close, and branch closures
// (design docs/design/qa-e2e.md §9.1–§9.5 ticket R9-4; blueprint/booking-slots.md §4.4, §7, §9 BK-9).
// Written by ticket #4288 (QA-R9-4); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/:
// - screens/admin/booking-slots/menu.ts:9-24
//   id: 'booking-slots' (:10), needs: 'BranchesManage' (:13)
// - screens/admin/booking-slots/BookingSlots.tsx:383-567 (smoke-admin-org.spec.ts:103-115)
//   .booking-slots-screen (:383), [role="alert"].error.field-error (:389)
// - Section 1 Slot Rule (BookingSlots.tsx:395-433, PUT /api/branches/{id}/booking-slot-rule :186-190):
//   input[name="slotMinutes"] (:399-406), input[name="capacity"] (:410-417),
//   input[name="horizonDays"] (:421-428), Save Rule button (:430-432, i18n.ts:15,65)
// - Section 2 Branch Closures (BookingSlots.tsx:436-479, POST /api/branches/{id}/closures :219-223,
//   POST /api/branches/{id}/closures/{date}/remove :244-248):
//   .closures-form (:438), input[name="closedOn"] (:441-448), input[name="reason"] (:452-459),
//   .closures-form button[type="button"] (:461-463, i18n.ts:17,67),
//   .closures-list .closure-item (:466-477), .closure-date (:470), .closure-reason (:471),
//   .closure-item button[type="button"] (:472-474, i18n.ts:18,68)
// - Section 3 Booking Slots (BookingSlots.tsx:482-523, POST /api/booking-slots/{id}/close :297-301,
//   POST /api/booking-slots/{id}/open :318-322):
//   .slots-list .slot-item[data-slot-id] (:484-519), .slot-date (:488), .slot-status (:492),
//   .slot-reserved (:493-495), Close button (:513-515, i18n.ts:13,63), Open button (:516-518, i18n.ts:14,64)
// - Section 4 Booking Policy (BookingSlots.tsx:526-566, PUT /api/booking-policy :370):
//   input[name="cancelCutoffMinutes"] (:531-538), input[name="noShowGraceMinutes"] (:541-549),
//   input[name="confirmExpiryMinutes"] (:553-560), Save Policy button (:562-564, i18n.ts:16,66)
// - shared/ui/AccountBar.tsx:143-170
//   .account-bar button[aria-haspopup="menu"], menuitemradio XC-B (POST /api/auth/session/branch)
// - screens/admin/AdminShell.tsx:279-301 (smoke-rbac-matrix.spec.ts:59-60,141-169)
//   nav.admin-nav a[href="#booking-slots"]
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, mePermissions, posted, shot, signIn, users, withLang } from './helpers';

const SAVE_RULE_BTN = /^(Save กฎการจอง|Save Rule)$/;
const SAVE_POLICY_BTN = /^(บันทึกนโยบาย|Save Policy)$/;
const CLOSE_SLOT_BTN = /^(ปิดรับจอง|Close slot)$/;
const OPEN_SLOT_BTN = /^(เปิดรับจอง|Open slot)$/;

const RULE_ERR: Record<'th' | 'en', string> = {
  th: 'ข้อมูลกฎไม่ถูกต้อง (ระยะเวลา 5–240 นาที, ความจุมากกว่า 0, ล่วงหน้า 1–90 วัน)',
  en: 'Rule validation failed (slot minutes 5-240, capacity > 0, horizon 1-90 days)',
};

const POLICY_ERR: Record<'th' | 'en', string> = {
  th: 'ข้อมูลนโยบายไม่ถูกต้อง (ยกเลิก >= 0, ผ่อนปรน >= 0, ยืนยัน > 0)',
  en: 'Policy validation failed (cutoff >= 0, grace >= 0, expiry > 0)',
};

// Date in Asia/Bangkok offset by `daysAhead` days (yyyy-mm-dd), matching flow-booking.spec.ts:7-10.
function bangkokDate(daysAhead: number): string {
  const now = new Date(Date.now() + 7 * 3600_000 + daysAhead * 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

// Waits for one PUT whose path matches while `act` runs; mirrors `posted` in helpers.ts:365-371.
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

// Ensures the admin session is targeting XC-B (MAIN) on /#booking-slots, returning the initial branch label.
async function openBookingSlotsOnMain(page: Page): Promise<string> {
  await page.goto('/#booking-slots');
  await expect(page.locator('.booking-slots-screen')).toBeVisible({ timeout: 20_000 });
  const branchButton = page.locator('.account-bar button[aria-haspopup="menu"]');
  const startBranch = ((await branchButton.innerText().catch(() => '')) || '').trim();
  if (!/XC-B/.test(startBranch)) {
    await branchButton.click();
    const switched = await posted(page, /^\/api\/auth\/session\/branch$/, () =>
      page.getByRole('menuitemradio', { name: /^XC-B$/ }).click(),
    );
    expect(switched.ok, `POST /api/auth/session/branch: ${switched.text}`).toBeTruthy();
    await page.waitForTimeout(1000);
    await page.goto('/#booking-slots');
    await expect(branchButton, 'session branch switched to XC-B').toContainText('XC-B', { timeout: 20_000 });
  }
  await expect(page.locator('input[name="slotMinutes"]')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('input[name="slotMinutes"]')).not.toHaveValue('', { timeout: 15_000 });
  return startBranch;
}

async function saveSlotRule(
  page: Page,
  rule: { slotMinutes: string; capacity: string; horizonDays: string },
): Promise<{ ok: boolean; text: string }> {
  await page.locator('input[name="slotMinutes"]').fill(rule.slotMinutes);
  await page.locator('input[name="capacity"]').fill(rule.capacity);
  await page.locator('input[name="horizonDays"]').fill(rule.horizonDays);
  return putted(page, /^\/api\/branches\/[^/]+\/booking-slot-rule$/, () =>
    page.getByRole('button', { name: SAVE_RULE_BTN }).click(),
  );
}

async function saveBookingPolicy(
  page: Page,
  policy: { cutoff: string; grace: string; expiry: string },
): Promise<{ ok: boolean; text: string }> {
  await page.locator('input[name="cancelCutoffMinutes"]').fill(policy.cutoff);
  await page.locator('input[name="noShowGraceMinutes"]').fill(policy.grace);
  await page.locator('input[name="confirmExpiryMinutes"]').fill(policy.expiry);
  return putted(page, /^\/api\/booking-policy$/, () =>
    page.getByRole('button', { name: SAVE_POLICY_BTN }).click(),
  );
}

test.describe.serial('R9-4 admin booking-slots: policy, slot rules, open/close, closures', () => {
  test('(a) happy path: bookingPolicySet, slotRuleSet, slotClose/slotOpen, closureAdd/closureRemove persist after reload and restore', async ({
    page,
  }) => {
    const marker = `qa-r9-4-${Date.now()}`;
    const minSlotDate = bangkokDate(7);
    // Closure date beyond the 14-day slot horizon (≥ 7 days ahead) so adding/removing the closure does not close existing slots.
    const closureDate = bangkokDate(21);

    await signIn(page, 'admin', '/#booking-slots');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('BranchesManage'), 'signed-in admin lacks BranchesManage');

    const startBranch = await openBookingSlotsOnMain(page);

    // Record the branch's current slot rule and organisation booking policy before any mutation (E-4).
    const origRule = {
      slotMinutes: (await page.locator('input[name="slotMinutes"]').inputValue()) || '30',
      capacity: (await page.locator('input[name="capacity"]').inputValue()) || '4',
      horizonDays: (await page.locator('input[name="horizonDays"]').inputValue()) || '14',
    };
    const origPolicy = {
      cutoff: (await page.locator('input[name="cancelCutoffMinutes"]').inputValue()) || '60',
      grace: (await page.locator('input[name="noShowGraceMinutes"]').inputValue()) || '15',
      expiry: (await page.locator('input[name="confirmExpiryMinutes"]').inputValue()) || '60',
    };

    let closedSlotId = '';

    try {
      // 1. bookingPolicySet (PUT /api/booking-policy, BookingSlots.tsx:333-377): update, reload, and read back.
      const nextPolicy = {
        cutoff: String(Number(origPolicy.cutoff) + 5),
        grace: String(Number(origPolicy.grace) + 5),
        expiry: String(Number(origPolicy.expiry) + 5),
      };
      const policySaved = await saveBookingPolicy(page, nextPolicy);
      console.log(`[R9-4] PUT /api/booking-policy ${policySaved.text}`);
      expect(policySaved.ok, `PUT /api/booking-policy accepted: ${policySaved.text}`).toBeTruthy();

      await page.reload();
      await expect(page.locator('input[name="cancelCutoffMinutes"]')).toHaveValue(nextPolicy.cutoff, {
        timeout: 15_000,
      });
      await expect(page.locator('input[name="noShowGraceMinutes"]')).toHaveValue(nextPolicy.grace, {
        timeout: 15_000,
      });
      await expect(page.locator('input[name="confirmExpiryMinutes"]')).toHaveValue(nextPolicy.expiry, {
        timeout: 15_000,
      });
      await shot(page, 'admin-booking-slots', 'r9-4-1-policy-saved');

      // 2. slotRuleSet (PUT /api/branches/{id}/booking-slot-rule, BookingSlots.tsx:156-199):
      //    keep slotMinutes unchanged so existing 30-min slots stay aligned, bump capacity by 1, reload, and read back.
      const nextRule = {
        slotMinutes: origRule.slotMinutes,
        capacity: String(Number(origRule.capacity) + 1),
        horizonDays: String(Math.max(Number(origRule.horizonDays), 14)),
      };
      const ruleSaved = await saveSlotRule(page, nextRule);
      console.log(`[R9-4] PUT /api/branches/*/booking-slot-rule ${ruleSaved.text}`);
      expect(ruleSaved.ok, `PUT booking-slot-rule accepted: ${ruleSaved.text}`).toBeTruthy();

      await page.reload();
      await expect(page.locator('input[name="slotMinutes"]')).toHaveValue(nextRule.slotMinutes, {
        timeout: 15_000,
      });
      await expect(page.locator('input[name="capacity"]')).toHaveValue(nextRule.capacity, {
        timeout: 15_000,
      });
      await expect(page.locator('input[name="horizonDays"]')).toHaveValue(nextRule.horizonDays, {
        timeout: 15_000,
      });
      await shot(page, 'admin-booking-slots', 'r9-4-2-rule-saved');

      // Restore the original slot rule now so generated future slots return to their seeded capacity (4).
      const ruleRestoredEarly = await saveSlotRule(page, origRule);
      expect(ruleRestoredEarly.ok, `PUT booking-slot-rule restore: ${ruleRestoredEarly.text}`).toBeTruthy();

      // 3. slotClose & slotOpen (POST /api/booking-slots/{id}/close and /open, BookingSlots.tsx:291-331):
      //    pick an OPEN slot with reserved: 0 on a date >= 7 days ahead so F1, F17 and F21 are untouched.
      const slotItems = page.locator('.slots-list .slot-item');
      await expect(slotItems.first(), 'at least one booking slot row').toBeVisible({ timeout: 15_000 });
      const slotCount = await slotItems.count();
      for (let i = 0; i < slotCount; i++) {
        const item = slotItems.nth(i);
        const dateText = ((await item.locator('.slot-date').innerText()) || '').trim();
        const statusText = ((await item.locator('.slot-status').innerText()) || '').trim();
        const reservedText = ((await item.locator('.slot-reserved').innerText()) || '').trim();
        if (dateText >= minSlotDate && statusText === 'OPEN' && /:\s*0$/.test(reservedText)) {
          closedSlotId = (await item.getAttribute('data-slot-id')) ?? '';
          if (closedSlotId) break;
        }
      }
      expect(closedSlotId, `an OPEN unreserved slot on or after ${minSlotDate}`).toBeTruthy();

      const targetSlot = page.locator(`.slots-list .slot-item[data-slot-id="${closedSlotId}"]`);
      const closeRes = await posted(
        page,
        new RegExp(`^/api/booking-slots/${encodeURIComponent(closedSlotId)}/close$`),
        () => targetSlot.getByRole('button', { name: CLOSE_SLOT_BTN }).click(),
      );
      console.log(`[R9-4] POST /api/booking-slots/${closedSlotId}/close ${closeRes.text}`);
      expect(closeRes.ok, `slotClose accepted: ${closeRes.text}`).toBeTruthy();

      await page.reload();
      await expect(
        page.locator(`.slots-list .slot-item[data-slot-id="${closedSlotId}"] .slot-status`),
        'slot status persisted as CLOSED after reload',
      ).toHaveText('CLOSED', { timeout: 15_000 });
      await shot(page, 'admin-booking-slots', 'r9-4-3-slot-closed');

      const openRes = await posted(
        page,
        new RegExp(`^/api/booking-slots/${encodeURIComponent(closedSlotId)}/open$`),
        () =>
          page
            .locator(`.slots-list .slot-item[data-slot-id="${closedSlotId}"]`)
            .getByRole('button', { name: OPEN_SLOT_BTN })
            .click(),
      );
      console.log(`[R9-4] POST /api/booking-slots/${closedSlotId}/open ${openRes.text}`);
      expect(openRes.ok, `slotOpen accepted: ${openRes.text}`).toBeTruthy();
      closedSlotId = '';

      await page.reload();
      await expect(
        page.locator(`.slots-list .slot-item .slot-status`).first(),
        'slots list loaded after reopen',
      ).toBeVisible({ timeout: 15_000 });
      await shot(page, 'admin-booking-slots', 'r9-4-4-slot-reopened');

      // 4. closureAdd & closureRemove (BookingSlots.tsx:201-257,438-478):
      //    add closure with run marker `qa-r9-4-<Date.now()>` on `closureDate` (>= 7 days ahead), reload, then remove and reload.
      await page.locator('input[name="closedOn"]').fill(closureDate);
      await page.locator('input[name="reason"]').fill(marker);
      const addClosureRes = await posted(page, /^\/api\/branches\/[^/]+\/closures$/, () =>
        page.locator('.closures-form button[type="button"]').click(),
      );
      console.log(`[R9-4] POST /api/branches/*/closures (${closureDate}, ${marker}) ${addClosureRes.text}`);
      expect(addClosureRes.ok, `closureAdd accepted: ${addClosureRes.text}`).toBeTruthy();

      await page.reload();
      const closureRow = page.locator('.closures-list .closure-item').filter({ hasText: marker });
      await expect(closureRow, 'added closure row visible after reload').toBeVisible({ timeout: 15_000 });
      await expect(closureRow.locator('.closure-date')).toHaveText(closureDate);
      await expect(closureRow.locator('.closure-reason')).toHaveText(marker);
      await shot(page, 'admin-booking-slots', 'r9-4-5-closure-added');

      const removeClosureRes = await posted(
        page,
        new RegExp(`^/api/branches/[^/]+/closures/${encodeURIComponent(closureDate)}/remove$`),
        () => closureRow.locator('button[type="button"]').click(),
      );
      console.log(`[R9-4] POST /api/branches/*/closures/${closureDate}/remove ${removeClosureRes.text}`);
      expect(removeClosureRes.ok, `closureRemove accepted: ${removeClosureRes.text}`).toBeTruthy();

      await page.reload();
      await expect(page.locator('.booking-slots-screen')).toBeVisible({ timeout: 15_000 });
      await expect(
        page.locator('.closures-list .closure-item').filter({ hasText: marker }),
        'removed closure row absent after reload',
      ).toHaveCount(0);
      await shot(page, 'admin-booking-slots', 'r9-4-6-closure-removed');
    } finally {
      // Restore slot open state, remove any leftover run-marker closure, and restore rule & policy (E-4).
      await page.goto('/#booking-slots').catch(() => {});
      await expect(page.locator('.booking-slots-screen')).toBeVisible({ timeout: 15_000 }).catch(() => {});

      if (closedSlotId) {
        const slotEl = page.locator(`.slots-list .slot-item[data-slot-id="${closedSlotId}"]`);
        if (await slotEl.isVisible().catch(() => false)) {
          await posted(
            page,
            new RegExp(`^/api/booking-slots/${encodeURIComponent(closedSlotId)}/open$`),
            () => slotEl.getByRole('button', { name: OPEN_SLOT_BTN }).click(),
          ).catch(() => {});
        }
      }

      const leftoverClosures = page.locator('.closures-list .closure-item').filter({ hasText: 'qa-r9-4-' });
      const leftoverCount = await leftoverClosures.count().catch(() => 0);
      for (let i = 0; i < leftoverCount; i++) {
        await leftoverClosures.first().locator('button[type="button"]').click().catch(() => {});
      }

      const ruleRestored = await saveSlotRule(page, origRule).catch((e) => ({ ok: false, text: String(e) }));
      const policyRestored = await saveBookingPolicy(page, origPolicy).catch((e) => ({ ok: false, text: String(e) }));
      console.log(`[R9-4 finally] restore rule=${ruleRestored.text} policy=${policyRestored.text}`);
      expect(ruleRestored.ok, `slot rule restored: ${ruleRestored.text}`).toBeTruthy();
      expect(policyRestored.ok, `booking policy restored: ${policyRestored.text}`).toBeTruthy();

      const original = startBranch.replace(/^สาขา\s*|^Branch\s*/i, '').trim();
      if (original && !/XC-B/.test(startBranch)) {
        await page.locator('.account-bar button[aria-haspopup="menu"]').click().catch(() => {});
        await page.getByRole('menuitemradio', { name: original }).click().catch(() => {});
      }
    }
  });

  test('(b) invalid inputs: 5 cases refused with worded th and en messages and typed values preserved', async ({
    page,
  }) => {
    await signIn(page, 'admin', '/#booking-slots');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('BranchesManage'), 'signed-in admin lacks BranchesManage');

    const startBranch = await openBookingSlotsOnMain(page);
    try {
      for (const lang of ['th', 'en'] as const) {
        await withLang(page, lang);
        await page.goto('about:blank');
        await page.goto('/#booking-slots');
        await expect(page.locator('input[name="slotMinutes"]')).not.toHaveValue('', { timeout: 15_000 });

        const alert = page.locator('[role="alert"].error.field-error');
        const saveRuleBtn = page.getByRole('button', { name: SAVE_RULE_BTN });
        const savePolicyBtn = page.getByRole('button', { name: SAVE_POLICY_BTN });

        // Case 1 (Slot Rule — capacity 0, BookingSlots.tsx:168-175): refused, input keeps '0'.
        await page.locator('input[name="slotMinutes"]').fill('30');
        await page.locator('input[name="capacity"]').fill('0');
        await page.locator('input[name="horizonDays"]').fill('14');
        await saveRuleBtn.click();
        await expect(alert, `case 1 capacity 0 (${lang})`).toHaveText(RULE_ERR[lang]);
        await expect(page.locator('input[name="capacity"]')).toHaveValue('0');

        // Case 2 (Booking Policy — non-integer / negative cutoff, BookingSlots.tsx:339-341): refused, input keeps '-5'.
        await page.locator('input[name="cancelCutoffMinutes"]').fill('-5');
        await page.locator('input[name="noShowGraceMinutes"]').fill('15');
        await page.locator('input[name="confirmExpiryMinutes"]').fill('60');
        await savePolicyBtn.click();
        await expect(alert, `case 2 cutoff -5 (${lang})`).toHaveText(POLICY_ERR[lang]);
        await expect(page.locator('input[name="cancelCutoffMinutes"]')).toHaveValue('-5');

        // Case 3 (Slot Rule — negative capacity, BookingSlots.tsx:168-175): refused, input keeps '-3'.
        await page.locator('input[name="capacity"]').fill('-3');
        await saveRuleBtn.click();
        await expect(alert, `case 3 capacity -3 (${lang})`).toHaveText(RULE_ERR[lang]);
        await expect(page.locator('input[name="capacity"]')).toHaveValue('-3');

        // Case 4 (Booking Policy — decimal cutoff not a whole number, BookingSlots.tsx:339-341): refused, input keeps '12.5'.
        await page.locator('input[name="cancelCutoffMinutes"]').fill('12.5');
        await savePolicyBtn.click();
        await expect(alert, `case 4 cutoff 12.5 (${lang})`).toHaveText(POLICY_ERR[lang]);
        await expect(page.locator('input[name="cancelCutoffMinutes"]')).toHaveValue('12.5');

        // Case 5 (Slot Rule — horizonDays 0 / end before start, BookingSlots.tsx:170-175): refused, input keeps '0'.
        await page.locator('input[name="capacity"]').fill('4');
        await page.locator('input[name="horizonDays"]').fill('0');
        await saveRuleBtn.click();
        await expect(alert, `case 5 horizonDays 0 (${lang})`).toHaveText(RULE_ERR[lang]);
        await expect(page.locator('input[name="horizonDays"]')).toHaveValue('0');

        await shot(page, 'admin-booking-slots', `r9-4-invalid-${lang}`);
      }
    } finally {
      const original = startBranch.replace(/^สาขา\s*|^Branch\s*/i, '').trim();
      if (original && !/XC-B/.test(startBranch)) {
        await page.locator('.account-bar button[aria-haspopup="menu"]').click().catch(() => {});
        await page.getByRole('menuitemradio', { name: original }).click().catch(() => {});
      }
    }
  });

  test('(c) permission: teller lacks BranchesManage — /#booking-slots is not offered and command routes refuse with 403', async ({
    page,
  }) => {
    test.skip(!hasAccount('teller'), 'no teller account in users.json: run scripts/reseed.sh');
    await signIn(page, 'teller', '/#booking-slots');

    // Menu gate (menu.ts:13 needs BranchesManage; smoke-rbac-matrix.spec.ts:59-60,141-169; AdminShell.tsx:279-301):
    // teller does not see nav link #booking-slots and .booking-slots-screen is not rendered.
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('nav.admin-nav a[href="#booking-slots"]')).toHaveCount(0);
    await expect(page.locator('.booking-slots-screen')).toHaveCount(0);

    // Direct command calls as teller are refused with 403 (roleNotAllowed / outOfScope).
    const branch = encodeURIComponent(users.branch);
    const dummySlot = '00000000-0000-0000-0000-000000000001';
    const closureDate = bangkokDate(21);

    const checks: Array<{ name: string; method: 'PUT' | 'POST'; url: string; data: Record<string, unknown> }> = [
      {
        name: 'bookingPolicySet',
        method: 'PUT',
        url: '/api/booking-policy',
        data: { cancelCutoffMinutes: 60, noShowGraceMinutes: 15, confirmExpiryMinutes: 60, branch: users.branch },
      },
      {
        name: 'slotRuleSet',
        method: 'PUT',
        url: `/api/branches/${branch}/booking-slot-rule`,
        data: { slotMinutes: 30, capacity: 4, horizonDays: 14, branch: users.branch },
      },
      {
        name: 'slotClose',
        method: 'POST',
        url: `/api/booking-slots/${dummySlot}/close`,
        data: { branch: users.branch },
      },
      {
        name: 'slotOpen',
        method: 'POST',
        url: `/api/booking-slots/${dummySlot}/open`,
        data: { branch: users.branch },
      },
      {
        name: 'closureRemove',
        method: 'POST',
        url: `/api/branches/${branch}/closures/${closureDate}/remove`,
        data: { branch: users.branch },
      },
    ];

    for (const c of checks) {
      const key = `qa-r9-4-teller-${c.name}-${Date.now()}`;
      const res =
        c.method === 'PUT'
          ? await page.request.put(c.url, { headers: { 'Idempotency-Key': key }, data: c.data })
          : await page.request.post(c.url, { headers: { 'Idempotency-Key': key }, data: c.data });
      const body = await res.text();
      console.log(`[R9-4 teller] ${c.method} ${c.url} -> ${res.status()} ${body.slice(0, 200)}`);
      expect(res.status(), `${c.name} refused for teller: ${body}`).toBe(403);
      expect(body, `${c.name} refusal code`).toMatch(/roleNotAllowed|outOfScope/);
    }
  });
});
