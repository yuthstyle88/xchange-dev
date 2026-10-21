// Flow F15 (docs/design/qa-e2e.md §4 F15):
//   Branch day on /staff floor (start service, cash target, attendance still-in, cut-off, close shift).
//   Written by ticket QA-F15 (#4157); this file is its whole File scope.
import { expect, test, type Page } from '@playwright/test';
import {
  clockIn,
  closeShift,
  ensureShiftClosed,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
} from './helpers';

const REQUIRED = ['teller', 'branchManager', 'finance'] as const;

const PHASE_READY = /(เตรียมกะแล้ว|Ready|^READY$)/i;
const PHASE_OPEN = /(เปิดรับรายการ|^Open$|^OPEN$)/i;
const PHASE_CUT_OFF = /(ตัดรายการแล้ว|Cut-off|^CUT_OFF$)/i;
const PHASE_CLOSED = /(ปิดกะ|^Closed$|^CLOSED$)/i;
const STILL_IN = /(ยังไม่ออก|Still in)/i;
const CONFIRM_BTN = /^(ยืนยัน|Confirm)$/;
const HOURS_REFUSAL = /เวลาทำการ|วันนี้สาขาปิด|เปิดบริการได้หลังเตรียมกะเสร็จ|ปิดรับรายการใหม่แล้ว|outside.*hours|service.*hours/i;

async function readTargetFigures(page: Page): Promise<Record<string, string>> {
  const out: Record<string, string> = {};
  const targetBox = page.getByTestId('floor-target');
  if (await targetBox.isVisible().catch(() => false)) {
    const items = targetBox.locator('.xc-floor-target-amount');
    const n = await items.count();
    for (let i = 0; i < n; i++) {
      const text = (await items.nth(i).innerText()).trim();
      const idx = text.indexOf(':');
      if (idx > 0) {
        const cur = text.slice(0, idx).trim();
        const amt = text.slice(idx + 1).trim();
        if (cur && amt) out[cur] = amt;
      }
    }
  }
  return out;
}

async function saveTargetFigures(page: Page, figures: Record<string, string>) {
  const form = page.getByTestId('floor-target-form');
  await expect(form).toBeVisible({ timeout: 20_000 });
  const inputs = form.locator('input[data-currency]');
  await expect(inputs.first(), 'currency inputs in floor-target-form').toBeVisible({ timeout: 20_000 });
  const count = await inputs.count();
  for (let i = 0; i < count; i++) {
    const input = inputs.nth(i);
    const cur = (await input.getAttribute('data-currency')) ?? '';
    await input.fill(figures[cur] ?? '');
  }
  return posted(page, /^\/api\/branch-day\/target$/, () =>
    form.getByTestId('floor-target-save').click(),
  );
}

test.describe.serial('F15 floor', () => {
  let teller: Page | undefined;
  let manager: Page | undefined;
  let finance: Page | undefined;
  let serviceStarted = false;
  let serviceCutOff = false;

  test.afterAll(async () => {
    try {
      if (teller && manager) {
        await signIn(teller, 'teller', '/staff');
        await signIn(manager, 'branchManager', '/staff');
        // Never cut off a day this spec did not start.
        if (serviceStarted && !serviceCutOff) {
          const cutoffBtn = manager.getByTestId('floor-cutoff');
          if (await cutoffBtn.isVisible({ timeout: 5_000 }).catch(() => false)) {
            const cut = await posted(manager, /^\/api\/branch-day\/cutoff$/, () => cutoffBtn.click());
            console.log(`[F15 afterAll] cutoff ${cut.text}`);
            if (cut.ok) serviceCutOff = true;
          }
        }
        await ensureShiftClosed(teller, manager);
      }
    } finally {
      await Promise.all([teller?.close(), manager?.close(), finance?.close()]);
    }
  });

  test('branch day on /staff floor: start service, cash target, attendance still-in, cut-off, close shift', async ({
    browser,
  }, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop', 'F15 floor runs on desktop project only');

    const missing = REQUIRED.filter((r) => !hasAccount(r));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')}`;
      console.log(`[F15] skip: ${reason}`);
      test.skip(true, reason);
    }

    teller = await browser.newPage();
    manager = await browser.newPage();
    finance = await browser.newPage();

    await signIn(teller, 'teller', '/staff');
    await signIn(manager, 'branchManager', '/staff');

    // Step 0 (teller + manager): openShift(teller, manager).
    await openShift(teller, manager);

    // Step 1 (manager): on /staff, read floor-phase. If not READY, skip and log the phase.
    await manager.reload();
    const phase = manager.getByTestId('floor-phase');
    await expect(phase).toBeVisible({ timeout: 20_000 });
    const currentPhase = (await phase.innerText()).trim();
    if (!PHASE_READY.test(currentPhase)) {
      const reason = `floor-phase is not READY (${currentPhase}); another day is running`;
      console.log(`[F15] skip: ${reason}`);
      test.skip(true, reason);
    }

    await expect(manager.getByTestId('floor-start')).toBeVisible({ timeout: 15_000 });
    await manager.getByTestId('floor-start').click();
    const dialog = manager.locator('dialog.xc-dialog');
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    const started = await posted(manager, /^\/api\/branch-day\/start$/, () =>
      dialog.getByRole('button', { name: CONFIRM_BTN }).click(),
    );
    console.log(`[F15] POST /api/branch-day/start ${started.text}`);
    if (!started.ok) {
      const errEl = manager.getByTestId('floor-error');
      const worded = (await errEl.isVisible().catch(() => false))
        ? (await errEl.innerText()).trim()
        : started.text;
      // Skip only on the worded hours refusal (#4130; CustodyError::ServiceNotReady / OutsideServiceHours,
      // api/crates/xc-shift/src/domain/custody/value_objects/custody_error.rs:57-58,69-71). Any other refusal fails.
      if (HOURS_REFUSAL.test(worded)) {
        console.log(`[F15] skip (outside MAIN hours): ${worded}`);
        test.skip(true, `branch-day/start refused outside hours: ${worded}`);
      }
      throw new Error(`branch-day/start refused: ${worded}`);
    }
    serviceStarted = true;
    expect(started.ok, `branch-day/start accepted: ${started.text}`).toBeTruthy();
    await expect(phase).toHaveText(PHASE_OPEN, { timeout: 15_000 });
    await shot(manager, 'staff', 'floor-1-open');

    // Step 2 & 2b (finance): read current floor-target figures, set USD to a run-unique figure,
    // verify after reload, and restore old figures (or empty target) in finally.
    await signIn(finance, 'finance', '/staff');
    const form = finance.getByTestId('floor-target-form');
    await expect(form).toBeVisible({ timeout: 20_000 });
    const usdInput = form.locator('input[data-currency="USD"]');
    await expect(usdInput, 'USD input in floor-target-form').toBeVisible({ timeout: 20_000 });

    const oldFigures = await readTargetFigures(finance);
    const uniqueUsd = String(10_000 + (Date.now() % 80_000));
    try {
      await usdInput.fill(uniqueUsd);
      const saved = await posted(finance, /^\/api\/branch-day\/target$/, () =>
        form.getByTestId('floor-target-save').click(),
      );
      console.log(`[F15] POST /api/branch-day/target (set USD=${uniqueUsd}) ${saved.text}`);
      expect(saved.ok, `target save accepted: ${saved.text}`).toBeTruthy();

      await finance.reload();
      const targetBox = finance.getByTestId('floor-target');
      await expect(targetBox).toBeVisible({ timeout: 20_000 });
      await expect(targetBox).toContainText(`USD: ${uniqueUsd}`);
      await shot(finance, 'staff', 'floor-2-target');
    } finally {
      const restored = await saveTargetFigures(finance, oldFigures);
      console.log(`[F15] POST /api/branch-day/target (restore) ${restored.text}`);
      expect(restored.ok, `target restore accepted: ${restored.text}`).toBeTruthy();
    }

    // Step 3 (teller): clockIn(teller); check attendance card's [data-attendance-id] row shows "still in".
    await clockIn(teller);
    await teller.reload();
    const attendanceRow = teller
      .locator('[data-card="attendance"] [data-attendance-id]')
      .filter({ hasText: STILL_IN })
      .first();
    await expect(attendanceRow, 'teller attendance row showing still in').toBeVisible({ timeout: 20_000 });
    await shot(teller, 'staff', 'floor-3-attendance');

    // Step 4 (manager): click floor-cutoff inside posted(manager, /^\/api\/branch-day\/cutoff$/, ...); floor-phase reads CUT_OFF.
    await manager.reload();
    const cutoffBtn = manager.getByTestId('floor-cutoff');
    await expect(cutoffBtn).toBeVisible({ timeout: 20_000 });
    const cut = await posted(manager, /^\/api\/branch-day\/cutoff$/, () => cutoffBtn.click());
    console.log(`[F15] POST /api/branch-day/cutoff ${cut.text}`);
    expect(cut.ok, `branch-day/cutoff accepted: ${cut.text}`).toBeTruthy();
    serviceCutOff = true;
    await expect(manager.getByTestId('floor-phase')).toHaveText(PHASE_CUT_OFF, { timeout: 15_000 });
    await shot(manager, 'staff', 'floor-4-cutoff');

    // Step 5 (teller + manager): closeShift(teller, manager); floor-phase reads CLOSED.
    await closeShift(teller, manager);
    await expect(manager.getByTestId('floor-phase')).toHaveText(PHASE_CLOSED, { timeout: 15_000 });
    await shot(manager, 'staff', 'floor-5-closed');
  });
});
