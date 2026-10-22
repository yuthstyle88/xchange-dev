// APP-7c (customer-app.md §9, §9.1): after a successful verify, C14 returns the customer to `?next=` when it is a
// same-origin path under /app (and not the sign-in page itself); anything else goes to /app (no open redirect).
// Up-front tests written by the FE lane controller claude/e8020026 before the fix. Do not edit, delete or weaken
// them; add tests in a file of your own.
//
// Pinned: `SignIn(props: { navigate; fetch?; search? })` — `search` defaults to `window.location.search`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { SignIn } from './SignIn';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

describe('APP-7c: sign-in returns to ?next=', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let verifyStatus: number;

  beforeAll(() => warmModules(() => import('./SignIn')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    verifyStatus = 200;
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
    if (url === '/api/auth/otp/request') return jsonResponse(200, { challengeId: 'ch-1' });
    if (url === '/api/auth/otp/verify') {
      return verifyStatus === 200
        ? jsonResponse(200, { user: { id: 'u-1', name: 'C' } })
        : jsonResponse(verifyStatus, { error: { code: 'otpInvalid', message: 'otpInvalid' } });
    }
    return jsonResponse(404, {});
  });

  function type(name: string, value: string): void {
    const input = host.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }
  function submit(): void {
    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
  }

  async function signInWith(search: string): Promise<ReturnType<typeof vi.fn>> {
    const navigate = vi.fn();
    const Comp = SignIn as unknown as (p: Record<string, unknown>) => ReturnType<typeof SignIn>;
    dispose = render(() => <Comp navigate={navigate} fetch={fetchFn} search={search} />, host);
    type('phone', '0812345678');
    submit();
    await vi.waitFor(() => expect(fetchFn.mock.calls.some((c) => String(c[0]) === '/api/auth/otp/request')).toBe(true));
    type('code', '123456');
    submit();
    return navigate;
  }

  async function landsOn(search: string, expected: string): Promise<void> {
    const navigate = await signInWith(search);
    await vi.waitFor(() => expect(navigate).toHaveBeenCalledTimes(1));
    expect(navigate).toHaveBeenCalledWith(expected);
  }

  it('next=/app/queue?branch=MAIN ⇒ back to exactly that page', async () => {
    await landsOn('?next=' + encodeURIComponent('/app/queue?branch=MAIN'), '/app/queue?branch=MAIN');
  });

  it('next=/app ⇒ /app', async () => {
    await landsOn('?next=%2Fapp', '/app');
  });

  it('no next ⇒ /app (unchanged behaviour)', async () => {
    await landsOn('', '/app');
  });

  it('an absolute URL ⇒ /app (no open redirect)', async () => {
    await landsOn('?next=' + encodeURIComponent('https://evil.example/app'), '/app');
  });

  it('a protocol-relative //host ⇒ /app', async () => {
    await landsOn('?next=' + encodeURIComponent('//evil.example/app'), '/app');
  });

  it('a backslash /\\host ⇒ /app', async () => {
    await landsOn('?next=' + encodeURIComponent('/\\evil.example/app'), '/app');
  });

  it('a path outside /app (/admin) ⇒ /app', async () => {
    await landsOn('?next=%2Fadmin', '/app');
  });

  it('a look-alike prefix (/application) ⇒ /app', async () => {
    await landsOn('?next=%2Fapplication', '/app');
  });

  it('the sign-in page itself ⇒ /app (no loop)', async () => {
    await landsOn('?next=' + encodeURIComponent('/app/sign-in?next=/app/queue'), '/app');
  });

  it('a failed verify navigates nowhere', async () => {
    verifyStatus = 400;
    const navigate = await signInWith('?next=' + encodeURIComponent('/app/queue?branch=MAIN'));
    await vi.waitFor(() => expect(fetchFn.mock.calls.filter((c) => String(c[0]) === '/api/auth/otp/verify').length).toBeGreaterThan(0));
    await new Promise((r) => setTimeout(r, 50));
    expect(navigate).not.toHaveBeenCalled();
  });
});
