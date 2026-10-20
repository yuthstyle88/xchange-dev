// QA-F4 — flow: rates draft and simulate, never submit (design docs/design/qa-e2e.md §3, §4; docs/design/blueprint/rates.md §4.4, §4.7).
// Written by ticket QA-F4 (#3875); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { posted, shot, signIn } from './helpers';

// Write routes on /#rates that must NEVER be called by this flow (design E-4; #3875 ruling):
// - POST /api/rate-drafts (RatesDraftCard.tsx:157, api/src/presentation/http/v3/commands/rate_draft_save.rs:79-80)
// - PUT /api/rate-drafts/{id} (RatesDraftCard.tsx:278, :330, rate_draft_save.rs:132-133)
// - POST /api/rate-drafts/{id}/submit (RatesDraftCard.tsx:210, rate_draft_submit.rs:54-55)
// - POST /api/rate-drafts/{id}/publish (RatesDraftCard.tsx:234, rate_draft_publish.rs:55-56)
// - POST /api/rate-policies (Rates.tsx:54, RatesDraftCard.tsx:310, rate_policy_save.rs:193-194)
// - POST /api/rate-references/fetch (RatesDraftCard.tsx:152, rate_reference_fetch.rs:47-48)
function isForbiddenWrite(method: string, path: string): boolean {
  if (method === 'POST' && path === '/api/rate-drafts') return true;
  if (method === 'PUT' && /^\/api\/rate-drafts\/[^/]+$/.test(path)) return true;
  if (/^\/api\/rate-drafts\/[^/]+\/(submit|publish)$/.test(path)) return true;
  if (method === 'POST' && path === '/api/rate-policies') return true;
  if (path === '/api/rate-references/fetch') return true;
  return false;
}

test.describe('QA-F4 flow: rates, simulate an existing draft, write nothing', () => {
  test('admin opens rates and simulates an open draft if one exists, sending no write', async ({ page }) => {
    // Step 1: signIn(page, 'admin', '/'), then page.goto('/#rates') (web/src/screens/admin/rates/menu.ts:6-12).
    await signIn(page, 'admin', '/');

    // Step 2: Record every request the page sends from here on (page.on('request')).
    const writeRequests: string[] = [];
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      const method = req.method();
      if (isForbiddenWrite(method, path)) {
        writeRequests.push(`${method} ${path}`);
      }
    });

    // Wait for readDrafts (Rates.tsx:137-160, GET /api/v3/views/rate_drafts at api/src/presentation/http/v3/views/rate_drafts.rs:34, :52)
    // so we know whether an open draft (status in DRAFT / PENDING_APPROVAL / APPROVED, Rates.tsx:145-147) is being loaded.
    const draftsViewPromise = page
      .waitForResponse(
        (r) => r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/rate_drafts',
        { timeout: 20_000 },
      )
      .catch(() => null);

    await page.goto('/#rates');

    // Screen root: .admin-rates-screen (web/src/screens/admin/rates/Rates.tsx:313).
    const screen = page.locator('.admin-rates-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    // Policy card root: .rates-section containing [data-testid="save-policy"] (web/src/screens/admin/rates/RatesPolicyCard.tsx:391, :402).
    const policyCard = screen.locator('.rates-section').filter({ has: page.getByTestId('save-policy') });
    await expect(policyCard).toBeVisible({ timeout: 20_000 });

    // Draft card root: .rates-section containing .btn-new-draft (web/src/screens/admin/rates/RatesDraftCard.tsx:355, :372).
    const draftCard = screen.locator('.rates-section').filter({ has: page.locator('.btn-new-draft') });
    await expect(draftCard).toBeVisible({ timeout: 20_000 });

    // Ensure readDrafts and any follow-up rate_draft detail read (Rates.tsx:148-156) have settled before checking .btn-simulate.
    const draftsRes = await draftsViewPromise;
    let openDraftId: string | undefined;
    if (draftsRes && draftsRes.ok()) {
      const body = (await draftsRes.json().catch(() => null)) as {
        data?: { drafts?: Array<{ id: string; status: string }> };
      } | null;
      const open = body?.data?.drafts?.find((d) => ['DRAFT', 'PENDING_APPROVAL', 'APPROVED'].includes(d.status));
      openDraftId = open?.id;
    }

    // Simulate button: .btn-simulate (web/src/screens/admin/rates/RatesDraftCard.tsx:391).
    const simulateBtn = draftCard.locator('button.btn-simulate');
    if (openDraftId) {
      await expect(simulateBtn).toBeVisible({ timeout: 20_000 });
    }

    // Step 3 & 4: If an open draft is shown (.btn-simulate visible, RatesDraftCard.tsx:391), simulate it; else log skip-step.
    if (await simulateBtn.isVisible()) {
      // POST /api/rate-drafts/{id}/simulate (RatesDraftCard.tsx:192, api/src/presentation/http/v3/commands/rate_draft_simulate.rs:63-64).
      const sim = await posted(page, /^\/api\/rate-drafts\/[^/]+\/simulate$/, () => simulateBtn.click());
      expect(sim.ok, `POST /api/rate-drafts/{id}/simulate: ${sim.text}`).toBeTruthy();

      const parsed = JSON.parse(sim.body) as {
        id: string;
        isInBound: boolean;
        outOfBound: Array<{ currency: string; side: string }>;
      };
      const draftId = parsed.id ?? openDraftId ?? 'unknown';

      // Assert that the simulated result renders: .in-bound-status (RatesDraftCard.tsx:417) when isInBound is true,
      // or .out-of-bound (RatesDraftCard.tsx:571) on out-of-bound draft lines when isInBound is false.
      if (parsed.isInBound) {
        await expect(
          draftCard.locator('.in-bound-status'),
          'in-bound simulate result (RatesDraftCard.tsx:417)',
        ).toBeVisible({ timeout: 15_000 });
      } else {
        await expect(
          draftCard.locator('.out-of-bound').first(),
          'out-of-bound simulate result (RatesDraftCard.tsx:571)',
        ).toBeVisible({ timeout: 15_000 });
      }

      console.log(`simulated draft id=${draftId}: ${sim.text}`);
      await shot(page, 'flow-rates', '1-simulated');
    } else {
      console.log('no open draft: simulate not exercised');
      test.info().annotations.push({
        type: 'skip-step',
        description: 'no open draft: simulate not exercised',
      });
      await shot(page, 'flow-rates', '1-no-open-draft');
    }

    // Step 5: Assert no write request was sent.
    expect(writeRequests, 'write requests sent on /#rates').toEqual([]);
  });
});
