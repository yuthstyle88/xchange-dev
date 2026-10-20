import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const POLICY_VIEW = '/api/v3/views/verification_policy';
const QUEUE_VIEW = '/api/v3/views/verification_queue';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('VerificationScreen policy request gate (#3906)', () => {
  let requested: string[] = [];
  let queueResponse: () => Response;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    requested = [];
    document.body.innerHTML = '';
    queueResponse = () => json({ version: 1, changed: true, data: [] }, 200);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(rawUrl, 'http://localhost');
      requested.push(url.pathname);

      if (url.pathname === '/api/v3/views/me' || url.pathname === '/api/v3/views/staff_me') {
        return json({ version: 1, changed: true, data: REVIEWER_SESSION });
      }
      if (url.pathname === QUEUE_VIEW) {
        return queueResponse();
      }
      if (url.pathname === POLICY_VIEW) {
        return json({
          version: 1,
          changed: true,
          data: {
            policyVersion: 1,
            branchReviewAllowed: true,
            acceptedDocumentTypes: ['PASSPORT'],
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

  it('requests verification_policy once when queue read succeeds (200)', async () => {
    queueResponse = () => json({ version: 1, changed: true, data: [] }, 200);
    await mountScreen();

    const policyRequests = requested.filter((p) => p === POLICY_VIEW);
    expect(policyRequests.length, 'verification_policy must be requested exactly once').toBe(1);
  });

  it('never requests verification_policy when queue read is refused with 403', async () => {
    queueResponse = () => json({ error: { code: 'roleNotAllowed', message: 'Not allowed' } }, 403);
    await mountScreen();

    expect(requested).not.toContain(POLICY_VIEW);
  });

  it('never requests verification_policy when queue read is refused with 400 roleNotAllowedHere', async () => {
    queueResponse = () => json({ error: { code: 'roleNotAllowedHere', message: 'Role not allowed here' } }, 400);
    await mountScreen();

    expect(requested).not.toContain(POLICY_VIEW);
  });
});
