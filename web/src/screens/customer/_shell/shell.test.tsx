// APP-7 (#998): customer app shell (design `customer-app.md` §4.5, §9.1 row APP-7; contract issuecomment-6137548682).
// Written before the implementation by test-writer gemini/warut-6 on claude/5edf7bb5's order.
// Do not edit, delete or weaken; add your own tests in another file.
//
// What this file pins:
// - `CustomerShell(props: { route: CustomerRoute; children?: JSX.Element })`
// - The current route is marked in the bottom navigation (e.g. aria-current="page" or active state)
// - Signed-in vs signed-out state comes from my_profile
// - The language switch shows TH / EN / 中文
// - th / en / zh translation tables have identical key sets with non-empty strings
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CustomerShell, type CustomerRoute } from './Shell';
import * as table from './i18n';

describe('CustomerShell (APP-7 #998)', () => {
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

  it('renders bottom navigation and marks the current route', async () => {
    const route: CustomerRoute = '/app';
    dispose = render(
      () => (
        <CustomerShell route={route}>
          <main>Home Page Content</main>
        </CustomerShell>
      ),
      host
    );

    const nav = host.querySelector('nav') ?? host.querySelector('[role="navigation"]');
    expect(nav, 'bottom navigation container exists').not.toBeNull();

    // Check that navigation items for key screens exist
    const links = Array.from(nav!.querySelectorAll('a, button'));
    expect(links.length, 'bottom navigation has items').toBeGreaterThan(0);

    // Find the nav link whose href equals the route and assert that it carries aria-current="page"
    const targetLink = links.find((el) => el.getAttribute('href') === '/app');
    expect(targetLink, 'nav link with href="/app" exists').toBeDefined();
    expect(targetLink!.getAttribute('aria-current'), 'link for /app has aria-current="page"').toBe('page');

    // Assert that exactly one nav element is current
    const currentItems = links.filter((el) => el.getAttribute('aria-current') === 'page');
    expect(currentItems.length, 'exactly one nav element is current').toBe(1);
  });

  it('marks /app/history as current when route="/app/history"', async () => {
    const route: CustomerRoute = '/app/history';
    dispose = render(
      () => (
        <CustomerShell route={route}>
          <main>History Page Content</main>
        </CustomerShell>
      ),
      host
    );

    const nav = host.querySelector('nav') ?? host.querySelector('[role="navigation"]');
    expect(nav, 'bottom navigation container exists').not.toBeNull();

    const links = Array.from(nav!.querySelectorAll('a, button'));
    const targetLink = links.find((el) => el.getAttribute('href') === '/app/history');
    expect(targetLink, 'nav link with href="/app/history" exists').toBeDefined();
    expect(targetLink!.getAttribute('aria-current'), 'link for /app/history has aria-current="page"').toBe('page');

    const currentItems = links.filter((el) => el.getAttribute('aria-current') === 'page');
    expect(currentItems.length, 'exactly one nav element is current').toBe(1);
  });

  it('signed-in state comes from my_profile: displays customer name when signed in', async () => {
    const mockProfile = {
      customerId: 'cust-108',
      customerNo: 'C10008',
      displayName: 'Somchai Dee',
      preferredLanguage: 'th',
      status: 'ACTIVE',
      consents: [],
    };

    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('my_profile')) {
        return new Response(JSON.stringify({ data: mockProfile }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('{}', { status: 200 });
    });

    dispose = render(
      () => (
        <CustomerShell route="/app">
          <div>Content</div>
        </CustomerShell>
      ),
      host
    );

    await vi.waitFor(() => {
      expect(host.textContent).toContain('Somchai Dee');
    });
  });

  it('signed-out state comes from my_profile: renders sign-in action when my_profile is 401', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('my_profile')) {
        return new Response(JSON.stringify({ error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('{}', { status: 200 });
    });

    dispose = render(
      () => (
        <CustomerShell route="/app">
          <div>Content</div>
        </CustomerShell>
      ),
      host
    );

    await vi.waitFor(() => {
      const links = Array.from(host.querySelectorAll('a, button'));
      const hasSignIn = links.some((el) => {
        const href = el.getAttribute('href') ?? '';
        const content = el.textContent ?? '';
        return (
          href.includes('sign-in') ||
          href.includes('login') ||
          content.includes('เข้าสู่ระบบ') ||
          content.includes('Sign In')
        );
      });
      expect(hasSignIn, 'renders sign-in action in signed-out state').toBe(true);
    });
  });

  it('the language switch shows TH / EN / 中文 for customer audience', async () => {
    dispose = render(
      () => (
        <CustomerShell route="/app">
          <div>Content</div>
        </CustomerShell>
      ),
      host
    );

    const buttons = Array.from(host.querySelectorAll('button')).map((b) => (b.textContent ?? '').trim());
    expect(buttons).toContain('TH');
    expect(buttons).toContain('EN');
    expect(buttons).toContain('中文');
  });

  it('th, en, and zh translation tables have identical key sets with non-empty strings', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    const zhKeys = Object.keys(table.zh).sort();
    expect(enKeys, 'en keys match th').toEqual(thKeys);
    expect(zhKeys, 'zh keys match th').toEqual(thKeys);
    expect(thKeys.length, 'translation table has keys').toBeGreaterThan(0);

    for (const [lang, tbl] of Object.entries({ th: table.th, en: table.en, zh: table.zh })) {
      for (const [k, v] of Object.entries(tbl as Record<string, string>)) {
        expect(typeof v, `${lang}.${k} is string`).toBe('string');
        expect(v.trim().length, `${lang}.${k} is non-empty`).toBeGreaterThan(0);
      }
    }
  });
});
