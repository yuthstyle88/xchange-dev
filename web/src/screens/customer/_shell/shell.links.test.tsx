// APP-7 (#3320): Customer shell links behavior tests (§8 Behaviour table).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CustomerShell } from './Shell';
import * as table from './i18n';

describe('CustomerShell bar links (#3320)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  const mockProfile = {
    customerId: 'cust-108',
    customerNo: 'C10008',
    displayName: 'Somchai Dee',
    preferredLanguage: 'th',
    status: 'ACTIVE',
    consents: [],
  };

  const mockSignedInFetch = () => {
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
  };

  const mockSignedOutFetch = () => {
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
  };

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

  it('signed in, route="/app": bar has links to /app/member, /app/rate-alerts, and name links to /app/profile; none is aria-current', async () => {
    mockSignedInFetch();
    dispose = render(() => <CustomerShell route="/app"><div>Content</div></CustomerShell>, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-customer-bar')).not.toBeNull();
      expect(host.querySelector('.xc-customer-name')).not.toBeNull();
    });

    const bar = host.querySelector('.xc-customer-bar')!;
    const profileLink = bar.querySelector('a[href="/app/profile"]');
    const memberLink = bar.querySelector('a[href="/app/member"]');
    const rateAlertsLink = bar.querySelector('a[href="/app/rate-alerts"]');

    expect(profileLink, 'profile link exists').not.toBeNull();
    expect(memberLink, 'member link exists').not.toBeNull();
    expect(rateAlertsLink, 'rate alerts link exists').not.toBeNull();

    expect(profileLink?.getAttribute('aria-current')).toBeNull();
    expect(memberLink?.getAttribute('aria-current')).toBeNull();
    expect(rateAlertsLink?.getAttribute('aria-current')).toBeNull();
  });

  it('signed in, route="/app/member": member link is aria-current="page"; no bottom-nav item is current', async () => {
    mockSignedInFetch();
    dispose = render(() => <CustomerShell route="/app/member"><div>Content</div></CustomerShell>, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-customer-bar a[href="/app/member"]')).not.toBeNull();
    });

    const memberLink = host.querySelector('.xc-customer-bar a[href="/app/member"]')!;
    expect(memberLink.getAttribute('aria-current')).toBe('page');

    const nav = host.querySelector('nav.xc-customer-nav')!;
    const navCurrent = nav.querySelectorAll('[aria-current="page"]');
    expect(navCurrent.length, 'no bottom-nav item is current').toBe(0);
  });

  it('signed in, route="/app/rate-alerts": rate-alerts link is aria-current="page"', async () => {
    mockSignedInFetch();
    dispose = render(() => <CustomerShell route="/app/rate-alerts"><div>Content</div></CustomerShell>, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-customer-bar a[href="/app/rate-alerts"]')).not.toBeNull();
    });

    const rateAlertsLink = host.querySelector('.xc-customer-bar a[href="/app/rate-alerts"]')!;
    expect(rateAlertsLink.getAttribute('aria-current')).toBe('page');

    const nav = host.querySelector('nav.xc-customer-nav')!;
    const navCurrent = nav.querySelectorAll('[aria-current="page"]');
    expect(navCurrent.length, 'no bottom-nav item is current').toBe(0);
  });

  it('signed in, route="/app/profile": name link is aria-current="page"', async () => {
    mockSignedInFetch();
    dispose = render(() => <CustomerShell route="/app/profile"><div>Content</div></CustomerShell>, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-customer-bar a[href="/app/profile"]')).not.toBeNull();
    });

    const profileLink = host.querySelector('.xc-customer-bar a[href="/app/profile"]')!;
    expect(profileLink.getAttribute('aria-current')).toBe('page');

    const nav = host.querySelector('nav.xc-customer-nav')!;
    const navCurrent = nav.querySelectorAll('[aria-current="page"]');
    expect(navCurrent.length, 'no bottom-nav item is current').toBe(0);
  });

  it('signed out (my_profile 401): none of the three links; sign-in link is rendered', async () => {
    mockSignedOutFetch();
    dispose = render(() => <CustomerShell route="/app"><div>Content</div></CustomerShell>, host);

    await vi.waitFor(() => {
      const signInLink = host.querySelector('.xc-customer-bar a[href="/app/sign-in"]');
      expect(signInLink).not.toBeNull();
    });

    const bar = host.querySelector('.xc-customer-bar')!;
    expect(bar.querySelector('a[href="/app/profile"]')).toBeNull();
    expect(bar.querySelector('a[href="/app/member"]')).toBeNull();
    expect(bar.querySelector('a[href="/app/rate-alerts"]')).toBeNull();
  });

  it('route="/app/journey": home bottom item is current', async () => {
    dispose = render(() => <CustomerShell route="/app/journey"><div>Content</div></CustomerShell>, host);

    const nav = host.querySelector('nav.xc-customer-nav')!;
    const homeLink = nav.querySelector('a[href="/app"]')!;
    expect(homeLink.getAttribute('aria-current')).toBe('page');

    const currentItems = nav.querySelectorAll('[aria-current="page"]');
    expect(currentItems.length).toBe(1);
  });

  it('keys: member, rateAlerts exist in th/en/zh and are non-empty', () => {
    for (const [lang, tbl] of Object.entries({ th: table.th, en: table.en, zh: table.zh })) {
      const record = tbl as Record<string, string>;
      expect(record.member, `${lang}.member exists`).toBeDefined();
      expect(typeof record.member, `${lang}.member is string`).toBe('string');
      expect(record.member.trim().length, `${lang}.member is non-empty`).toBeGreaterThan(0);

      expect(record.rateAlerts, `${lang}.rateAlerts exists`).toBeDefined();
      expect(typeof record.rateAlerts, `${lang}.rateAlerts is string`).toBe('string');
      expect(record.rateAlerts.trim().length, `${lang}.rateAlerts is non-empty`).toBeGreaterThan(0);
    }
  });
});
