// QA-F16 — flow: place and release a customer hold on /staff › holds
// (design docs/design/qa-e2e.md §4 row F16; cash-shift-transfer.md:87-88,296; xchange-v3-wave3-staff.md:53,167,188-189).
// Written by ticket #4158 (QA-F16); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/staff/:
// - holds/HoldsCard.tsx:153-236
//   [data-card="holds"], [data-testid="holds-form"], [data-testid="holds-key"], [data-testid="holds-find"],
//   [data-customer-id="<id>"], [data-testid="holds-reason"], [data-testid="holds-place"],
//   [data-hold-id="<id>"], [data-testid="holds-release"]
// - parts/ReasonDialog.tsx:31-50
//   dialog.xc-dialog, input[type="text"], .xc-dialog-actions button
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users, type Role } from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['customer', 'teller', 'branchManager'];
const MARKER_PREFIX = 'e2e-F16';

interface StaffHoldRow {
  id: string;
  customerId: string;
  customerName: string | null;
  reason: string;
  status: string;
  at: string;
}

const holdsCard = (page: Page) => page.locator('[data-card="holds"]');

async function readStaffHolds(page: Page): Promise<StaffHoldRow[]> {
  const res = await page.request.get('/api/v3/views/staff_holds');
  const text = `${res.status()} ${(await res.text().catch(() => '')).slice(0, 300)}`;
  expect(res.ok(), `GET /api/v3/views/staff_holds: ${text}`).toBeTruthy();
  const body = (await res.json()) as { data?: StaffHoldRow[] };
  return Array.isArray(body.data) ? body.data : [];
}

async function releaseHoldOnStaff(manager: Page, holdId: string, note: string): Promise<void> {
  const card = holdsCard(manager);
  await expect(card, '[data-card="holds"] visible for branchManager').toBeVisible({ timeout: 20_000 });
  const row = card.locator(`[data-hold-id="${holdId}"]`);
  if (!(await row.isVisible().catch(() => false))) {
    await manager.reload();
  }
  await expect(row, `hold row [data-hold-id="${holdId}"]`).toBeVisible({ timeout: 20_000 });
  await expect(row, `hold ${holdId} is OPEN before release`).toContainText('OPEN', { timeout: 15_000 });

  await row.getByTestId('holds-release').click();
  const dialog = card.locator('dialog.xc-dialog');
  await expect(dialog, 'release ReasonDialog (dialog.xc-dialog)').toBeVisible({ timeout: 15_000 });
  await dialog.locator('input[type="text"]').fill(note);

  const relRes = await posted(
    manager,
    new RegExp(`^/api/customer-holds/${encodeURIComponent(holdId)}/release$`),
    () => dialog.getByRole('button', { name: /^(ยืนยัน|Confirm)$/ }).click(),
  );
  console.log(`[F16] POST /api/customer-holds/${holdId}/release ${relRes.text}`);
  expect(relRes.ok, `POST /api/customer-holds/${holdId}/release: ${relRes.text}`).toBeTruthy();

  await expect(dialog, 'release dialog closed after confirm').toBeHidden({ timeout: 15_000 });
  await expect(row, `hold row [data-hold-id="${holdId}"] shows RELEASED`).toContainText('RELEASED', {
    timeout: 20_000,
  });
}

test.describe.serial('F16 customer hold', () => {
  test('teller places a customer hold on /staff and branch manager releases it', async ({ browser }) => {
    const missing = REQUIRED_ROLES.filter((role) => !hasAccount(role));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')} (run scripts/reseed.sh)`;
      console.log(`[F16] skip: ${reason}`);
      test.skip(true, reason);
    }

    const customer = await browser.newPage({ baseURL: users.baseUrl });
    const teller = await browser.newPage({ baseURL: users.baseUrl });
    const manager = await browser.newPage({ baseURL: users.baseUrl });

    const runMarker = `${MARKER_PREFIX}-${Date.now()}`;
    let customerId = '';
    let placedHoldId = '';
    let holdPlaced = false;
    let holdReleased = false;
    const cleanupFailures: string[] = [];

    try {
      // Step 0 (customer): read my_profile.customerId (signed-in request.get('/api/v3/views/my_profile')) -> non-empty.
      await signIn(customer, 'customer', '/app');
      const profRes = await customer.request.get('/api/v3/views/my_profile');
      const profRaw = await profRes.text().catch(() => '');
      const profText = `${profRes.status()} ${profRaw.slice(0, 300)}`;
      expect(profRes.ok(), `GET /api/v3/views/my_profile: ${profText}`).toBeTruthy();
      try {
        const parsed = JSON.parse(profRaw) as {
          data?: { customerId?: string };
          customerId?: string;
        };
        customerId = String(parsed.data?.customerId ?? parsed.customerId ?? '').trim();
      } catch {
        // leave customerId empty so the assertion fails with the response text
      }
      expect(customerId, `my_profile.customerId must be non-empty (${profText})`).not.toBe('');
      console.log(`[F16] customerId=${customerId}`);

      // Step 1 (manager): pre-clean any OPEN hold of that customer whose reason carries the spec's marker (e2e-F16).
      await signIn(manager, 'branchManager', '/staff');
      await expect(holdsCard(manager), 'manager holds card visible on /staff').toBeVisible({ timeout: 20_000 });

      const initialHolds = await readStaffHolds(manager);
      const leftoverHolds = initialHolds.filter(
        (h) => h.customerId === customerId && h.status === 'OPEN' && h.reason.includes(MARKER_PREFIX),
      );
      for (const leftover of leftoverHolds) {
        console.log(`[F16] pre-clean: releasing leftover OPEN hold ${leftover.id} (${leftover.reason})`);
        await releaseHoldOnStaff(manager, leftover.id, `${MARKER_PREFIX} pre-clean`);
      }

      const afterPreClean = await readStaffHolds(manager);
      expect(
        afterPreClean.filter(
          (h) => h.customerId === customerId && h.status === 'OPEN' && h.reason.includes(MARKER_PREFIX),
        ),
        'none left OPEN with the marker after pre-clean',
      ).toEqual([]);

      // Step 2 (teller on /staff › holds): holds-key = the id, holds-find, [data-customer-id=<id>],
      // holds-reason = marker + run id, holds-place, then posted(/^\/api\/customer-holds$/) -> ok; [data-hold-id] shows OPEN.
      await signIn(teller, 'teller', '/staff');
      const tCard = holdsCard(teller);
      await expect(tCard.getByTestId('holds-form'), 'teller holds-form visible on /staff').toBeVisible({
        timeout: 20_000,
      });

      await tCard.getByTestId('holds-key').fill(customerId);
      await tCard.getByTestId('holds-find').click();

      const customerRow = tCard.locator(`[data-customer-id="${customerId}"]`);
      await expect(customerRow, `search result row [data-customer-id="${customerId}"]`).toBeVisible({
        timeout: 20_000,
      });
      await customerRow.click();
      await expect(customerRow).toHaveAttribute('aria-pressed', 'true');

      await tCard.getByTestId('holds-reason').fill(runMarker);
      holdPlaced = true;
      const placeRes = await posted(teller, /^\/api\/customer-holds$/, () =>
        tCard.getByTestId('holds-place').click(),
      );
      console.log(`[F16] POST /api/customer-holds ${placeRes.text}`);
      expect(placeRes.ok, `POST /api/customer-holds: ${placeRes.text}`).toBeTruthy();

      try {
        const parsed = JSON.parse(placeRes.body) as { id?: string };
        placedHoldId = String(parsed.id ?? '').trim();
      } catch {
        // fallback to matching row by runMarker
      }

      const tellerHoldRow = placedHoldId
        ? tCard.locator(`[data-hold-id="${placedHoldId}"]`)
        : tCard.locator('[data-hold-id]').filter({ hasText: runMarker }).first();
      await expect(tellerHoldRow, '[data-hold-id] row after placing hold').toBeVisible({ timeout: 20_000 });
      if (!placedHoldId) {
        placedHoldId = (await tellerHoldRow.getAttribute('data-hold-id')) ?? '';
      }
      expect(placedHoldId, 'placed hold id').not.toBe('');
      await expect(tellerHoldRow).toContainText(runMarker);
      await expect(tellerHoldRow, '[data-hold-id] row shows OPEN').toContainText('OPEN');
      await shot(teller, 'staff', 'f16-1-hold-placed');

      // Step 3 (manager): holds-release on that row, a reason in dialog.xc-dialog, confirm -> ok; the row shows RELEASED.
      await manager.reload();
      await releaseHoldOnStaff(manager, placedHoldId, `${runMarker} release`);
      holdReleased = true;
      await shot(manager, 'staff', 'f16-2-hold-released');
    } finally {
      // Step 4 (finally): if step 2 placed a hold and step 3 did not run or failed, release it as the manager.
      try {
        if (holdPlaced) {
          if (!holdReleased) {
            await signIn(manager, 'branchManager', '/staff');
            await manager.reload();
            const currentHolds = await readStaffHolds(manager);
            const openRunHolds = currentHolds.filter(
              (h) =>
                h.status === 'OPEN' &&
                ((placedHoldId && h.id === placedHoldId) || h.reason.includes(runMarker)),
            );
            for (const h of openRunHolds) {
              console.log(`[F16] finally: releasing OPEN hold ${h.id} (${h.reason})`);
              await releaseHoldOnStaff(manager, h.id, `${runMarker} finally-release`);
            }
          }
          const finalHolds = await readStaffHolds(manager);
          expect(
            finalHolds.filter((h) => h.status === 'OPEN' && h.reason.includes(runMarker)),
            "no OPEN hold with this run's marker",
          ).toEqual([]);
        }
      } catch (err) {
        console.error('[F16] finally cleanup failed:', err);
        cleanupFailures.push(String(err));
      } finally {
        await Promise.all([customer.close(), teller.close(), manager.close()]);
      }
    }

    expect(cleanupFailures, 'finally cleanup must leave no OPEN hold with this run marker').toEqual([]);
  });
});
