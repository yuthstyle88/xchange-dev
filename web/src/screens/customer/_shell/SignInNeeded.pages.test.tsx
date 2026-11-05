// A signed-out visit (every customer read answers 401) shows the wine "Sign in" button to the sign-in page with
// `next=` back to the page, instead of the server's "please sign in" text with only a Retry button.
// One case per page that read its 401 as a plain load error: bookings, member, notifications, rate alerts,
// verification.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from '../bookings/Bookings';
import { MemberScreen } from '../member/Member';
import { NotificationsScreen } from '../notifications/Notifications';
import { RateAlertsScreen } from '../rate-alerts/RateAlerts';
import { VerificationScreen } from '../verification/VerificationScreen';
import { isSignedOut, signInHref, watchSignedOut } from './SignInNeeded';
import * as table from './i18n';

function urlOf(input: RequestInfo | URL): string {
  return typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
}

/** Signed out: every member read and command answers 401; the public reads answer empty. */
async function signedOutFetch(input: RequestInfo | URL): Promise<Response> {
  const url = urlOf(input);
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  if (url.includes('/api/public/host')) return json(200, { org: 'XC' });
  if (url.includes('/api/public/')) return json(200, { branches: [], rates: [] });
  if (url.includes('/api/')) return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
  return json(200, {});
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.spyOn(globalThis, 'fetch').mockImplementation(signedOutFetch as typeof fetch);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
  window.history.replaceState(null, '', '/');
});

async function expectSignInButton(path: string): Promise<void> {
  await vi.waitFor(() => {
    expect(host.querySelector('[data-signin-needed]'), 'the sign-in prompt is shown').not.toBeNull();
  });
  const link = host.querySelector<HTMLAnchorElement>('[data-signin-needed] a.xc-c-primary');
  expect(link, 'a primary sign-in button').not.toBeNull();
  expect(link!.textContent).toBe(table.th.signIn);
  expect(link!.getAttribute('href')).toBe(`/app/sign-in?next=${encodeURIComponent(path)}`);
  expect(host.querySelector('[data-signin-needed]')!.textContent).toContain('กรุณาเข้าสู่ระบบ');
}

describe('signed-out customer pages show a sign-in button with next=', () => {
  it('bookings', async () => {
    window.history.replaceState(null, '', '/app/bookings');
    dispose = render(() => <BookingsScreen fetch={(i, init) => fetch(i, init)} />, host);
    await expectSignInButton('/app/bookings');
    expect(host.querySelector('[data-action="retry"]'), 'no retry for a signed-out read').toBeNull();
  });

  it('member', async () => {
    window.history.replaceState(null, '', '/app/member');
    dispose = render(() => <MemberScreen />, host);
    await expectSignInButton('/app/member');
    expect(host.querySelector('.xc-member-retry-btn')).toBeNull();
  });

  it('notifications', async () => {
    window.history.replaceState(null, '', '/app/notifications');
    dispose = render(() => <NotificationsScreen />, host);
    await expectSignInButton('/app/notifications');
    expect(host.querySelector('.xc-notifications-retry-btn')).toBeNull();
  });

  it('rate alerts: the prompt replaces the create form', async () => {
    window.history.replaceState(null, '', '/app/rate-alerts');
    dispose = render(() => <RateAlertsScreen />, host);
    await expectSignInButton('/app/rate-alerts');
    expect(host.querySelector('.xc-create-alert-section')).toBeNull();
    expect(host.querySelector('.xc-alerts-retry-btn')).toBeNull();
  });

  it('verification', async () => {
    window.history.replaceState(null, '', '/app/verification?case=1');
    dispose = render(() => <VerificationScreen />, host);
    await expectSignInButton('/app/verification?case=1');
  });
});

describe('the parts', () => {
  it('isSignedOut is true only for a 401', () => {
    expect(isSignedOut({ status: 401 })).toBe(true);
    expect(isSignedOut({ status: 403 })).toBe(false);
    expect(isSignedOut(new Error('x'))).toBe(false);
    expect(isSignedOut(undefined)).toBe(false);
  });

  it('signInHref encodes next once', () => {
    expect(signInHref('/app/bookings?x=1')).toBe('/app/sign-in?next=%2Fapp%2Fbookings%3Fx%3D1');
  });

  it('watchSignedOut passes answers and errors through and reports only a 401', async () => {
    const seen: string[] = [];
    const client = {
      ok: async () => 'answer',
      out: async () => {
        throw Object.assign(new Error('signed out'), { status: 401 });
      },
      down: async () => {
        throw Object.assign(new Error('down'), { status: 503 });
      },
      plain: () => 7,
    };
    const watched = watchSignedOut(client, () => seen.push('401'));
    await expect(watched.ok()).resolves.toBe('answer');
    await expect(watched.out()).rejects.toThrow('signed out');
    await expect(watched.down()).rejects.toThrow('down');
    expect(watched.plain()).toBe(7);
    expect(seen).toEqual(['401']);
  });
});
