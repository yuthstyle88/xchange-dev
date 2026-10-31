// Session branch switching and verification helpers for e2e specs (refs #5800).
// Admin screens ignore ?branch= in URLs (AdminShell.tsx:243-261); writes act on the session branch
// or command JSON body (command.rs:336-365). Switching branch writes users.last_branch_id
// (session_service.rs:1090-1099), so tests that switch must always switch back in finally.
import { expect, type Page } from '@playwright/test';
import { posted } from './helpers';

export interface ActingBranch { branch: string; branchName: string; branches: Array<{ id: string; code: string; name: string }> }

/** GET /api/v3/views/me → data.branch / branchName / branches (Me, web/src/shared/api/generated/common.ts:11). */
export async function actingBranch(page: Page): Promise<ActingBranch> {
  const res = await page.request.get('/api/v3/views/me');
  expect(res.ok(), `GET /api/v3/views/me: ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as {
    data?: {
      branch?: string;
      branchName?: string;
      branches?: Array<{ id: string; code: string; name: string }>;
    };
  };
  return {
    branch: body.data?.branch ?? '',
    branchName: body.data?.branchName ?? '',
    branches: body.data?.branches ?? [],
  };
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Fails (expect) unless the page acts on `branchId`: me.branch === branchId, `.account-bar` shows `(สาขา|Branch|分店) <branchName>`
 *  (AccountBar.tsx:155-170), and, when `header.menu-heading p.subtitle` exists, it contains `· <branchName>` (AdminShell.tsx:675-681).
 *  `label` goes into every failure message. */
export async function assertActingBranch(page: Page, branchId: string, label: string): Promise<void> {
  const me = await actingBranch(page);
  expect(me.branch, `${label}: acting branch id`).toBe(branchId);
  const expectedName = me.branches.find((b) => b.id === branchId)?.name ?? me.branchName;
  const branchPattern = new RegExp(`(?:สาขา|Branch|分店)\\s+${escapeRegex(expectedName)}`);
  await expect(page.locator('.account-bar'), `${label}: .account-bar shows ${expectedName}`).toContainText(branchPattern, {
    timeout: 20_000,
  });
  const subtitle = page.locator('header.menu-heading p.subtitle');
  if (await subtitle.count() > 0) {
    await expect(subtitle.first(), `${label}: header.menu-heading p.subtitle contains · ${expectedName}`).toContainText(
      `· ${expectedName}`,
      { timeout: 20_000 },
    );
  }
}

/** Switches the session through the account bar (AccountBar.tsx:99-117,155-187). Writes users.last_branch_id
 *  (session_service.rs:1090-1099). Returns the branch it switched from. No-op (no POST) when already on `branchId`.
 *  Fails when `branchId` is not in me.branches, or when POST /api/auth/session/branch is refused. Ends with assertActingBranch. */
export async function switchSessionBranch(page: Page, branchId: string): Promise<string> {
  const current = await actingBranch(page);
  const initialBranch = current.branch;
  if (current.branch === branchId) {
    await assertActingBranch(page, branchId, `switchSessionBranch (already on ${branchId})`);
    return initialBranch;
  }

  const target = current.branches.find((b) => b.id === branchId);
  const seen = current.branches.map((b) => `${b.id} (${b.name})`).join(', ');
  expect(target, `switchSessionBranch: branch ${branchId} not in me.branches [${seen}]`).toBeTruthy();
  if (!target) {
    throw new Error(`switchSessionBranch: branch ${branchId} not in me.branches [${seen}]`);
  }

  const branchButton = page.locator('.account-bar button[aria-haspopup="menu"]');
  await expect(branchButton, 'account bar branch button').toBeVisible({ timeout: 20_000 });
  if ((await branchButton.getAttribute('aria-expanded')) !== 'true') {
    await branchButton.click();
  }

  const menuItem = page.getByRole('menuitemradio', { name: target.name, exact: true });
  await expect(menuItem, `menu item for branch ${target.name}`).toBeVisible({ timeout: 10_000 });
  const switched = await posted(page, /^\/api\/auth\/session\/branch$/, () => menuItem.click());
  expect(switched.ok, `POST /api/auth/session/branch: ${switched.text} ${switched.body}`).toBeTruthy();

  await page.waitForLoadState();
  const branchPattern = new RegExp(`(?:สาขา|Branch|分店)\\s+${escapeRegex(target.name)}`);
  await expect(page.locator('.account-bar'), `account bar showing ${target.name}`).toContainText(branchPattern, {
    timeout: 20_000,
  });

  await assertActingBranch(page, branchId, `switchSessionBranch (${target.name})`);
  return initialBranch;
}
