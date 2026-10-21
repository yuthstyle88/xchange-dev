// #4301: written before the fix by the ticket author claude/studio-claude-4 (sub-controller under XC FE claude/e92db6).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// The customer screen gated on `my_profile.permissions`, a field the real `my_profile` answer does not have
// (api/src/presentation/http/v3/views/my_profile.rs answers `{ version, changed, data: MyProfileData }`, and
// `MyProfileData` has no `permissions`). `can()` then threw, and the screen showed `error.network` without ever
// asking for `my_verification`. `my_verification` refuses a caller with no customer id itself
// (my_verification.rs:237-240, `403 outOfScope`), so that refusal is the gate. These tests mock the server's real
// shapes:
//   - [required] with the real `my_profile` answer, the screen asks for `my_verification` and renders;
//   - [required] `my_verification` 403 `outOfScope` → the blocked panel with `accessDenied`, no screen;
//   - [required] `my_verification` 500 → no blocked panel, and the screen's own load error shows (not `accessDenied`).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { makeMyProfileData, makeMyVerificationViewAnswer } from '../../../shared/testing/fixtures';
import { mount } from './main';

const WAIT_OPTS = { timeout: 500, interval: 20 };

interface Backend {
  verificationStatus?: number;
  verificationCode?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function setupFetch(b: Backend = {}): string[] {
  const calls: string[] = [];
  const handler = async (input: RequestInfo | URL): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, 'http://localhost').pathname;
    calls.push(path);
    if (path === '/api/v3/views/my_profile') {
      // The server's real answer: a ViewAnswer wrapper around MyProfileData, with no `permissions`.
      return json({ version: 1, changed: true, data: makeMyProfileData({ displayName: 'Customer Test' }) });
    }
    if (path === '/api/v3/views/my_verification') {
      const status = b.verificationStatus ?? 200;
      if (status !== 200) {
        return json({ error: { code: b.verificationCode ?? 'serverError', message: 'refused' } }, status);
      }
      // my_verification.rs:264-267 answers the data bare.
      return json(makeMyVerificationViewAnswer({ case: null, readiness: 'NONE' }));
    }
    return json({ error: { code: 'notFound', message: 'not mocked' } }, 404);
  };
  vi.stubGlobal('fetch', vi.fn(handler));
  return calls;
}

describe('#4301 customer verification gate', () => {
  let host: HTMLElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('[required] with the real my_profile shape, asks for my_verification and renders the screen', async () => {
    const calls = setupFetch();
    dispose = mount(host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-verification-screen="true"]')).not.toBeNull();
    }, WAIT_OPTS);
    expect(calls).toContain('/api/v3/views/my_verification');
    expect(host.querySelector('.xc-verification-blocked')).toBeNull();
  });

  it('[required] my_verification 403 outOfScope shows the blocked panel with accessDenied and no screen', async () => {
    setupFetch({ verificationStatus: 403, verificationCode: 'outOfScope' });
    dispose = mount(host);

    await vi.waitFor(() => {
      const blocked = host.querySelector('.xc-verification-blocked');
      expect(blocked).not.toBeNull();
      expect(blocked!.textContent).toContain('Access denied. Please sign in with a customer account.');
    }, WAIT_OPTS);
    expect(host.querySelector('[data-verification-screen="true"]')).toBeNull();
  });

  it('[required] my_verification 500 is a load error, not an access refusal', async () => {
    const calls = setupFetch({ verificationStatus: 500, verificationCode: 'serverError' });
    dispose = mount(host);

    await vi.waitFor(() => {
      expect(calls).toContain('/api/v3/views/my_verification');
    }, WAIT_OPTS);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-verification-screen="true"]')).not.toBeNull();
    }, WAIT_OPTS);
    expect(host.textContent ?? '').not.toContain('Access denied. Please sign in with a customer account.');
  });
});
