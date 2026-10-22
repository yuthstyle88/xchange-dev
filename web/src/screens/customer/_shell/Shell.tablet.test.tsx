// BT-F4 (kiosk-shared-phone §7.11 W-D12): on a paired tablet the customer shell shows no links on flow pages.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CustomerShell } from './Shell';

describe('CustomerShell tablet flow (BT-F4 §7.11 W-D12 #4688)', () => {
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

  const pairedResponse = {
    paired: true,
    tabletId: 'bt-1',
    branchCode: 'MAIN',
    name: 'Front',
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

  it('S1: tabletFlow, GET /api/branch-tablet/me => 200 paired: no a[href] in .xc-customer-bar, no nav.xc-customer-nav, children render', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/branch-tablet/me')) {
        return new Response(JSON.stringify(pairedResponse), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
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
        <CustomerShell route="/app/queue" tabletFlow>
          <div id="flow-children">Queue Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar).not.toBeNull();
      expect(bar!.querySelectorAll('a[href]').length).toBe(0);
    });

    expect(host.querySelector('nav.xc-customer-nav')).toBeNull();
    expect(host.querySelector('#flow-children')).not.toBeNull();
    expect(host.textContent).toContain('Queue Content');
  });

  it('S2: tabletFlow, me => {paired:false}: the bar and the nav render exactly as today', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/branch-tablet/me')) {
        return new Response(JSON.stringify({ paired: false }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
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
        <CustomerShell route="/app/queue" tabletFlow>
          <div>Queue Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar?.querySelector('a.xc-customer-name')).not.toBeNull();
      expect(host.querySelector('nav.xc-customer-nav')).not.toBeNull();
    });

    const nav = host.querySelector('nav.xc-customer-nav')!;
    expect(nav.querySelectorAll('a').length).toBe(5);
    const bar = host.querySelector('.xc-customer-bar')!;
    expect(bar.querySelector('a[href="/app/notifications"]')).not.toBeNull();
  });

  it('S3: tabletFlow, me => 404 (BE not deployed) or network failure: as S2', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/branch-tablet/me')) {
        return new Response('Not Found', { status: 404 });
      }
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
        <CustomerShell route="/app/queue" tabletFlow>
          <div>Queue Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar?.querySelector('a.xc-customer-name')).not.toBeNull();
      expect(host.querySelector('nav.xc-customer-nav')).not.toBeNull();
    });

    const nav = host.querySelector('nav.xc-customer-nav')!;
    expect(nav.querySelectorAll('a').length).toBe(5);
    const bar = host.querySelector('.xc-customer-bar')!;
    expect(bar.querySelector('a[href="/app/notifications"]')).not.toBeNull();
  });

  it('S4: no tabletFlow (e.g. /app/bookings), even on a paired tablet: as today, and no request to /api/branch-tablet/me', async () => {
    const urlsFetched: string[] = [];
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      urlsFetched.push(url);
      if (url.includes('/api/branch-tablet/me')) {
        return new Response(JSON.stringify(pairedResponse), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
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
        <CustomerShell route="/app/bookings">
          <div>Bookings Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar?.querySelector('a.xc-customer-name')).not.toBeNull();
      expect(host.querySelector('nav.xc-customer-nav')).not.toBeNull();
    });

    expect(urlsFetched.some((u) => u.includes('/api/branch-tablet/me'))).toBe(false);
    const nav = host.querySelector('nav.xc-customer-nav')!;
    expect(nav.querySelectorAll('a').length).toBe(5);
  });

  it('S5: tabletFlow, paired, signed out (my_profile 401): the bar sign-in link is not rendered', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/branch-tablet/me')) {
        return new Response(JSON.stringify(pairedResponse), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
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
        <CustomerShell route="/app/queue" tabletFlow>
          <div>Queue Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      const bar = host.querySelector('.xc-customer-bar');
      expect(bar).not.toBeNull();
      expect(bar!.querySelectorAll('a[href]').length).toBe(0);
    });

    const signInLink = host.querySelector('.xc-customer-bar a[href="/app/sign-in"]');
    expect(signInLink).toBeNull();
    expect(host.querySelector('.xc-customer-signin')).toBeNull();
    expect(host.querySelector('nav.xc-customer-nav')).toBeNull();
  });

  it('S6: tabletFlow, me not answered yet: the bar and the nav render as today until the read answers (§2)', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/branch-tablet/me')) {
        return new Promise<Response>(() => {});
      }
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
        <CustomerShell route="/app/queue" tabletFlow>
          <div>Queue Content</div>
        </CustomerShell>
      ),
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-customer-bar a.xc-customer-name')).not.toBeNull();
      expect(host.querySelector('nav.xc-customer-nav')).not.toBeNull();
    });
  });
});
