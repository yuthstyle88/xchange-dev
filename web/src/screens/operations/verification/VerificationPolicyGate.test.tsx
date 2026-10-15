// F02's at-branch form follows the organisation's verification policy (follow-up of #1400 / PR 1871; the view is
// #1878, design `docs/design/blueprint/verification.md` §4.5 `verification_policy`). Written before the
// implementation; do not edit, delete or weaken.
//
// The view answers the shared envelope `{version, changed, data}` with
// `data = {policyVersion, branchReviewAllowed, acceptedDocumentTypes, documentsRequired}`; an organisation with no
// policy is refused `400 noVerificationPolicy`. The form shows **only** when that field is `true`; every other answer
// (false, refused, unreachable) keeps it hidden with the worded line, as PR 1871 does today (fail closed).
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { en as screenEn, th as screenTh } from './i18n';
import { VerificationScreen } from './VerificationScreen';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const POLICY_VIEW = '/api/v3/views/verification_policy';

type PolicyAnswer =
  | { kind: 'ok'; branchReviewAllowed: boolean }
  | { kind: 'refused'; status: number; code: string }
  | { kind: 'network' };

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('F02 at-branch form follows verification_policy.branchReviewAllowed (#1878)', () => {
  let policy: PolicyAnswer = { kind: 'ok', branchReviewAllowed: false };
  let requested: string[] = [];
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    requested = [];
    document.body.innerHTML = '';

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(rawUrl, 'http://localhost');
      requested.push(url.pathname);

      if (url.pathname === '/api/v3/views/me' || url.pathname === '/api/v3/views/staff_me') {
        return json({ version: 1, changed: true, data: REVIEWER_SESSION });
      }
      if (url.pathname === '/api/v3/views/verification_queue') {
        return json({ version: 1, changed: true, data: [] });
      }
      if (url.pathname === POLICY_VIEW) {
        if (policy.kind === 'network') {
          throw new TypeError('Failed to fetch');
        }
        if (policy.kind === 'refused') {
          return json({ error: { code: policy.code, message: policy.code } }, policy.status);
        }
        return json({
          version: 2,
          changed: true,
          data: {
            policyVersion: 2,
            branchReviewAllowed: policy.branchReviewAllowed,
            acceptedDocumentTypes: ['PASSPORT', 'NATIONAL_ID'],
            documentsRequired: 1,
          },
        });
      }
      return json({ error: { code: 'notFound', message: 'Not found' } }, 404);
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function mountScreen(): Promise<void> {
    dispose?.();
    document.body.innerHTML = '';
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <VerificationScreen />, host);
    for (let i = 0; i < 15; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  const form = () => document.querySelector('[data-testid="branch-verification-form"]');
  const disabledLine = () => document.querySelector('[data-testid="branch-verification-disabled"]');

  it('reads the verification_policy view', async () => {
    await mountScreen();
    expect(requested, 'the screen must read the policy view').toContain(POLICY_VIEW);
  });

  it('shows the at-branch form when the policy allows it, and no disabled line', async () => {
    policy = { kind: 'ok', branchReviewAllowed: true };
    await mountScreen();
    expect(form(), 'branchReviewAllowed = true shows the form').not.toBeNull();
    expect(disabledLine()).toBeNull();
  });

  it('keeps the form hidden with the worded line when the policy forbids it, in TH and EN', async () => {
    policy = { kind: 'ok', branchReviewAllowed: false };
    setLang('th');
    await mountScreen();
    expect(form()).toBeNull();
    expect(disabledLine()?.textContent).toBe(screenTh.branchReviewDisabled);

    setLang('en');
    await mountScreen();
    expect(form()).toBeNull();
    expect(disabledLine()?.textContent).toBe(screenEn.branchReviewDisabled);
  });

  it('keeps the form hidden when the organisation has no policy (400 noVerificationPolicy)', async () => {
    policy = { kind: 'refused', status: 400, code: 'noVerificationPolicy' };
    await mountScreen();
    expect(form()).toBeNull();
    expect(disabledLine()?.textContent).toBe(screenTh.branchReviewDisabled);
  });

  it('keeps the form hidden when the policy cannot be read (network failure): fail closed', async () => {
    policy = { kind: 'network' };
    await mountScreen();
    expect(form()).toBeNull();
    expect(disabledLine()).not.toBeNull();
  });

  it('keeps the form hidden when the policy view is refused for scope (403 outOfScope)', async () => {
    policy = { kind: 'refused', status: 403, code: 'outOfScope' };
    await mountScreen();
    expect(form()).toBeNull();
    expect(disabledLine()).not.toBeNull();
  });
});
