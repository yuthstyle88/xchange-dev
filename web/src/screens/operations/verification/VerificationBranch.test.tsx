import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
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

describe('VF-9: At-branch intake fail-closed (V17) & wordedError compliance', () => {
  let queueError: { status: number; code: string; message: string } | null = null;
  let throwNetworkOnQueue = false;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    queueError = null;
    throwNetworkOnQueue = false;
    document.body.innerHTML = '';

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const parsedUrl = new URL(rawUrl, 'http://localhost');

      if (parsedUrl.pathname === '/api/v3/views/me' || parsedUrl.pathname === '/api/v3/views/staff_me') {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: REVIEWER_SESSION,
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }

      if (parsedUrl.pathname === '/api/v3/views/verification_queue') {
        if (throwNetworkOnQueue) {
          throw new TypeError('Failed to fetch');
        }
        if (queueError) {
          return new Response(
            JSON.stringify({ error: { code: queueError.code, message: queueError.message } }),
            { status: queueError.status, headers: { 'Content-Type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: [],
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ error: { code: 'notFound', message: 'Not found' } }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' },
      });
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

  it('with the real me shape, hides the at-branch form and shows the worded disabled line in TH and EN', async () => {
    setLang('th');
    await mountScreen();

    expect(document.querySelector('[data-testid="branch-verification-form"]')).toBeNull();
    expect(document.querySelector('form')).toBeNull();
    const disabledTh = document.querySelector('[data-testid="branch-verification-disabled"]');
    expect(disabledTh).not.toBeNull();
    expect(disabledTh?.textContent).toBe(screenTh.branchReviewDisabled);

    setLang('en');
    await mountScreen();

    expect(document.querySelector('[data-testid="branch-verification-form"]')).toBeNull();
    expect(document.querySelector('form')).toBeNull();
    const disabledEn = document.querySelector('[data-testid="branch-verification-disabled"]');
    expect(disabledEn).not.toBeNull();
    expect(disabledEn?.textContent).toBe(screenEn.branchReviewDisabled);
  });

  it('wordedError: 500 with keyed non-server code shows error.serverError; network error shows error.network; unknown non-5xx shows server sentence', async () => {
    // 1. 500 with keyed non-server code
    queueError = { status: 500, code: 'invalidRequest', message: 'internal trace' };
    await mountScreen();
    expect(document.querySelector('[data-error]')?.textContent).toContain(sharedTh['error.serverError']);

    // 2. Network TypeError in TH and EN
    queueError = null;
    throwNetworkOnQueue = true;
    setLang('th');
    await mountScreen();
    expect(document.querySelector('[data-error]')?.textContent).toContain(sharedTh['error.network']);

    setLang('en');
    await mountScreen();
    expect(document.querySelector('[data-error]')?.textContent).toContain(sharedEn['error.network']);

    // 3. Unknown non-5xx refusal shows server sentence
    throwNetworkOnQueue = false;
    queueError = { status: 422, code: 'customUnkeyedRefusal', message: 'ข้อความปฏิเสธเฉพาะของเซิร์ฟเวอร์' };
    setLang('th');
    await mountScreen();
    expect(document.querySelector('[data-error]')?.textContent).toContain('ข้อความปฏิเสธเฉพาะของเซิร์ฟเวอร์');
  });
});
