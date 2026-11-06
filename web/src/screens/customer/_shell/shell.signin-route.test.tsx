// APP-7 (#6314) Sign-in and register route behavior in CustomerShell
// S1: /app/sign-in -> no "เข้าสู่ระบบ" chip in the header; no tab has aria-current
// S2: /app (home), signed out -> the chip shows and Home is current, as today
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { CustomerShell } from './Shell';

describe('CustomerShell sign-in route (#6314)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    host.remove();
    vi.restoreAllMocks();
  });

  it('S1: /app/sign-in when signed out has no sign-in chip and marks no tab as current', async () => {
    vi.spyOn(ApiClient.prototype, 'view').mockRejectedValue({ status: 401 });

    dispose = render(
      () => (
        <CustomerShell route="/app/sign-in">
          <main>Sign In Content</main>
        </CustomerShell>
      ),
      host,
    );

    // Wait for profile resource to resolve to signed out
    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar).not.toBeNull();
    });

    // Check that there is no sign-in chip in header
    const signInChip = host.querySelector('a.xc-customer-signin');
    expect(signInChip, 'no sign-in chip in the header on /app/sign-in').toBeNull();

    // Check that no tab has aria-current
    const nav = host.querySelector('nav.xc-customer-nav');
    expect(nav, 'bottom nav exists').not.toBeNull();
    const currentTabs = nav!.querySelectorAll('a[aria-current]');
    expect(currentTabs.length, 'no tab has aria-current on /app/sign-in').toBe(0);
  });

  it('S2: /app (home) when signed out shows sign-in chip and marks Home as current', async () => {
    vi.spyOn(ApiClient.prototype, 'view').mockRejectedValue({ status: 401 });

    dispose = render(
      () => (
        <CustomerShell route="/app">
          <main>Home Content</main>
        </CustomerShell>
      ),
      host,
    );

    // Wait for profile resource to resolve to signed out
    await vi.waitFor(() => {
      const signInChip = host.querySelector('a.xc-customer-signin');
      expect(signInChip, 'sign-in chip shows on /app when signed out').not.toBeNull();
      expect(signInChip!.getAttribute('href')).toBe('/app/sign-in');
    });

    const nav = host.querySelector('nav.xc-customer-nav');
    expect(nav, 'bottom nav exists').not.toBeNull();
    const homeLink = nav!.querySelector('a[href="/app"]');
    expect(homeLink, 'home tab link exists').not.toBeNull();
    expect(homeLink!.getAttribute('aria-current'), 'Home tab has aria-current="page"').toBe('page');

    const currentTabs = nav!.querySelectorAll('a[aria-current]');
    expect(currentTabs.length, 'only Home tab is marked current').toBe(1);
  });

  it('register page /app/sign-up when signed out has no sign-in chip and marks no tab as current', async () => {
    vi.spyOn(ApiClient.prototype, 'view').mockRejectedValue({ status: 401 });

    dispose = render(
      () => (
        <CustomerShell route="/app/sign-up">
          <main>Sign Up Content</main>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar).not.toBeNull();
    });

    const signInChip = host.querySelector('a.xc-customer-signin');
    expect(signInChip, 'no sign-in chip in header on /app/sign-up').toBeNull();

    const nav = host.querySelector('nav.xc-customer-nav');
    expect(nav, 'bottom nav exists').not.toBeNull();
    const currentTabs = nav!.querySelectorAll('a[aria-current]');
    expect(currentTabs.length, 'no tab has aria-current on /app/sign-up').toBe(0);
  });

  it('marks no tab as current on /app/sign-in even when customer profile is signed in', async () => {
    vi.spyOn(ApiClient.prototype, 'view').mockResolvedValue({
      data: { displayName: 'Somchai Dee' },
      version: 1,
      changed: false,
    });

    dispose = render(
      () => (
        <CustomerShell route="/app/sign-in">
          <main>Sign In Content</main>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const nameLink = host.querySelector('a.xc-customer-name');
      expect(nameLink).not.toBeNull();
    });

    const nav = host.querySelector('nav.xc-customer-nav');
    const currentTabs = nav!.querySelectorAll('a[aria-current]');
    expect(currentTabs.length, 'no tab has aria-current').toBe(0);
  });
});
