// APP-7d (#4648, customer-app.md §9, §9.1): after a successful register, C14 sign-up returns the customer to `?next=`
// when it passes `SignIn.tsx`'s shared `nextTarget` check, or to `/app` when there is none; the "have an account" link
// carries `next` to `/app/sign-in?next=…`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { SignUp } from './SignUp';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

describe('APP-7d (#4648): sign-up returns to ?next= and keeps next on the sign-in link', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let registerStatus: number;

  beforeAll(() => warmModules(() => import('./SignUp')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    registerStatus = 200;
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (url === '/api/app/register') {
      return registerStatus === 200
        ? jsonResponse(200, { ok: true })
        : jsonResponse(registerStatus, { error: { code: 'alreadyRegistered', message: 'alreadyRegistered' } });
    }
    return jsonResponse(404, {});
  });

  function renderSignUp(search: string): ReturnType<typeof vi.fn> {
    const navigate = vi.fn();
    dispose = render(() => <SignUp navigate={navigate} fetch={fetchFn} search={search} />, host);
    return navigate;
  }

  async function landsOn(search: string, expected: string): Promise<void> {
    const navigate = renderSignUp(search);
    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
    await vi.waitFor(() => expect(navigate).toHaveBeenCalledTimes(1));
    expect(navigate).toHaveBeenCalledWith(expected);
  }

  it('(1) ?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26from%3Dsign-in ⇒ a successful register navigates to /app/queue?branch=MAIN&from=sign-in', async () => {
    await landsOn(
      '?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26from%3Dsign-in',
      '/app/queue?branch=MAIN&from=sign-in',
    );
  });

  it('(2a) no next ⇒ /app, as today', async () => {
    await landsOn('', '/app');
  });

  it('(2b) unsafe next (another origin https://evil.example/app) ⇒ /app', async () => {
    await landsOn('?next=' + encodeURIComponent('https://evil.example/app'), '/app');
  });

  it('(2c) unsafe next (protocol-relative //evil.example/app) ⇒ /app', async () => {
    await landsOn('?next=' + encodeURIComponent('//evil.example/app'), '/app');
  });

  it('(2d) unsafe next (/app/sign-in…) ⇒ /app', async () => {
    await landsOn('?next=' + encodeURIComponent('/app/sign-in?next=/app/queue'), '/app');
  });

  it('(3) the "have an account" link keeps next and falls back to /app/sign-in when absent or unsafe', () => {
    renderSignUp('?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26from%3Dsign-in');
    const linkWithNext = host.querySelector<HTMLAnchorElement>('.xc-sign-in-switch a')!;
    expect(linkWithNext.getAttribute('href')).toBe(
      '/app/sign-in?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26from%3Dsign-in',
    );

    dispose?.();
    host.innerHTML = '';
    renderSignUp('');
    const linkBare = host.querySelector<HTMLAnchorElement>('.xc-sign-in-switch a')!;
    expect(linkBare.getAttribute('href')).toBe('/app/sign-in');

    dispose?.();
    host.innerHTML = '';
    renderSignUp('?next=' + encodeURIComponent('https://evil.example/app'));
    const linkUnsafe = host.querySelector<HTMLAnchorElement>('.xc-sign-in-switch a')!;
    expect(linkUnsafe.getAttribute('href')).toBe('/app/sign-in');
  });

  it('a failed register navigates nowhere', async () => {
    registerStatus = 409;
    const navigate = renderSignUp('?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26from%3Dsign-in');
    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
    await vi.waitFor(() =>
      expect(fetchFn.mock.calls.filter((c) => String(c[0]) === '/api/app/register').length).toBeGreaterThan(0),
    );
    await new Promise((r) => setTimeout(r, 50));
    expect(navigate).not.toHaveBeenCalled();
  });
});
