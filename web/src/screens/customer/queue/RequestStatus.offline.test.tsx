// Up-front tests for SF-b, the RequestStatus half (docs/design/web-silent-failures.md §2 SF-2, B1). Do not edit,
// delete or weaken; add tests in a file of your own.
//
// RequestStatus shows a failed read: no good read yet ⇒ a worded "unavailable" line, never a blank label; a good read
// earlier ⇒ the last status stays plus a "last updated hh:mm" line; a 401 ⇒ the sign-in link; after `expiresAt` a
// 404 stays silent (onExpired only). The harness is RequestStatus.test.tsx's, copied. Tests run with TZ=UTC
// (vitest.config.ts), so 20:00:00Z shows as 20:00.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { en, th } from './i18n';
import type { WebRequestStatus } from './queue-api';
import { RequestStatus } from './RequestStatus';

const EXPIRES = '2026-10-21T21:00:00Z';
const BEFORE = new Date('2026-10-21T20:00:00Z');
const AFTER = new Date('2026-10-21T21:00:01Z');
const INTERVAL = 3_000;
const SIGN_IN = '/app/sign-in?next=%2Fapp%2Fqueue%3Fbranch%3DMAIN%26request%3Dk-1';

let reads: string[];
let views: Array<() => Response>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let onExpired: ReturnType<typeof vi.fn<() => void>>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** A changed `my_web_request` answer; `version` is W4a's opaque string. */
function view(status: WebRequestStatus, displayNumber: string | null, version: string) {
  return () =>
    json(200, {
      version,
      changed: true,
      data: {
        requestId: 'k-1',
        status,
        expiresAt: status === 'QUOTED' ? EXPIRES : null,
        branch: { code: 'MAIN', name: 'XC-B' },
        type: 'BUY',
        currency: 'USD',
        amount: '100',
        displayNumber,
        ticketsAhead: displayNumber ? 2 : null,
      },
    });
}

const notFound = () => json(404, { error: { code: 'notFound', message: 'notFound' } });
const serverError = () => json(500, { error: { code: 'serverError', message: 'serverError' } });
const unauthorized = () => json(401, { error: { code: 'signIn', message: 'signIn' } });
/** The network is down: fetch itself rejects. */
const offline = (): Response => {
  throw new TypeError('Failed to fetch');
};

const fetchFn = async (input: string): Promise<Response> => {
  const u = new URL(input, 'http://localhost');
  if (u.pathname !== '/api/v3/views/my_web_request') return notFound();
  reads.push(u.search);
  const next = views.shift();
  return next ? next() : json(200, { version: 'last', changed: false });
};

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function tick(): Promise<void> {
  await vi.advanceTimersByTimeAsync(INTERVAL);
  await flush();
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

const shownStatus = () => q('[data-request-status]')?.getAttribute('data-request-status') ?? null;
const errorLine = () => q('[data-status-error]');
const signInLink = () => q<HTMLAnchorElement>('a[data-status-signin]');

/** `Status:` with nothing after it. */
function labelIsBlank(): boolean {
  const label = q('.xc-queue-status-label');
  if (!label) return false;
  return /:\s*$/.test(label.textContent ?? '');
}

async function mount(): Promise<void> {
  dispose = render(
    () => (
      <RequestStatus
        client={new ApiClient(fetchFn)}
        branchId="b-id-7"
        requestId="k-1"
        expiresAt={EXPIRES}
        intervalMs={INTERVAL}
        status={null}
        onExpired={onExpired}
        signInHref={SIGN_IN}
      />
    ),
    host,
  );
  await flush();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(BEFORE);
  setAudience('customer');
  setLang('th');
  reads = [];
  views = [];
  onExpired = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

describe('SF-b: RequestStatus, the first read fails', () => {
  it('a 500 first: the worded unavailable line, never a blank label', async () => {
    views = [serverError];
    await mount();
    expect(reads).toHaveLength(1);
    expect(errorLine()?.getAttribute('data-status-error')).toBe('unavailable');
    expect(errorLine()?.textContent ?? '').toContain(th.statusUnavailable);
    expect(labelIsBlank(), 'the label is not "Status:" with nothing after it').toBe(false);
    expect(signInLink()).toBeNull();
  });

  it('the network down first: the same worded unavailable line', async () => {
    views = [offline];
    await mount();
    expect(errorLine()?.getAttribute('data-status-error')).toBe('unavailable');
    expect(errorLine()?.textContent ?? '').toContain(th.statusUnavailable);
    expect(labelIsBlank()).toBe(false);
  });

  it('the line is in the page language', async () => {
    setLang('en');
    views = [serverError];
    await mount();
    expect(errorLine()?.textContent ?? '').toContain(en.statusUnavailable);
  });

  it('the next good read removes the line and shows the status', async () => {
    views = [serverError, view('QUOTED', null, '5.00aa')];
    await mount();
    expect(errorLine()).not.toBeNull();
    await tick();
    expect(shownStatus()).toBe('QUOTED');
    expect(errorLine()).toBeNull();
  });
});

describe('SF-b: RequestStatus, a later read fails', () => {
  it('the last status, number and place in line stay, plus "last updated hh:mm"', async () => {
    views = [view('WAITING', 'K042', '6.00bb'), serverError];
    await mount();
    expect(shownStatus()).toBe('WAITING');
    expect(errorLine()).toBeNull();
    await tick();
    expect(reads).toHaveLength(2);
    expect(shownStatus()).toBe('WAITING');
    expect(q('[data-request-status]')?.textContent).toContain(th.status_WAITING);
    expect(q('[data-display-number]')?.textContent).toContain('K042');
    expect(host.textContent).toContain(th.ahead.replace('{count}', '2'));
    expect(errorLine()?.getAttribute('data-status-error')).toBe('stale');
    expect(errorLine()?.textContent ?? '').toContain(th.statusStale.replace('{time}', '20:00'));
    expect(signInLink()).toBeNull();
  });

  it('"last updated" is the last good read, not the failed one', async () => {
    views = [view('QUOTED', null, '5.00aa'), serverError];
    await mount();
    vi.setSystemTime(new Date('2026-10-21T20:41:00Z'));
    await tick();
    expect(errorLine()?.textContent ?? '').toContain(th.statusStale.replace('{time}', '20:00'));
    expect(errorLine()?.textContent ?? '').not.toContain('20:41');
  });

  it('the network down later: the same stale line', async () => {
    views = [view('QUOTED', null, '5.00aa'), offline];
    await mount();
    await tick();
    expect(shownStatus()).toBe('QUOTED');
    expect(errorLine()?.getAttribute('data-status-error')).toBe('stale');
  });

  it('the next good read removes the line', async () => {
    views = [view('QUOTED', null, '5.00aa'), serverError, view('WAITING', 'K042', '6.00bb')];
    await mount();
    await tick();
    expect(errorLine()).not.toBeNull();
    await tick();
    expect(shownStatus()).toBe('WAITING');
    expect(errorLine()).toBeNull();
  });
});

describe('SF-b: RequestStatus, 401', () => {
  it('a 401 first: the sign-in link, and no error line', async () => {
    views = [unauthorized];
    await mount();
    expect(signInLink()?.getAttribute('href')).toBe(SIGN_IN);
    expect(signInLink()?.textContent ?? '').toContain(th.statusSignIn);
    expect(errorLine()).toBeNull();
  });

  it('a 401 after a good read: the last status stays, and the sign-in link shows', async () => {
    views = [view('WAITING', 'K042', '6.00bb'), unauthorized];
    await mount();
    await tick();
    expect(shownStatus()).toBe('WAITING');
    expect(signInLink()?.getAttribute('href')).toBe(SIGN_IN);
    expect(errorLine()).toBeNull();
  });
});

describe('SF-b: RequestStatus, after expiresAt (guards: silent as today)', () => {
  it('a 404 after expiresAt calls onExpired once, with no error line and no sign-in link', async () => {
    views = [view('QUOTED', null, '5.00aa'), notFound, notFound, notFound];
    await mount();
    vi.setSystemTime(AFTER);
    await tick();
    expect(onExpired).toHaveBeenCalledTimes(1);
    expect(errorLine()).toBeNull();
    expect(signInLink()).toBeNull();
    await vi.advanceTimersByTimeAsync(INTERVAL * 3);
    await flush();
    expect(onExpired).toHaveBeenCalledTimes(1);
    expect(errorLine()).toBeNull();
  });
});
