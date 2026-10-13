import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../i18n';
import { meOf } from '../../test-support/me';
import { AppShell } from './AppShell';
import { CustomerShell } from '../../screens/customer/_shell/Shell';

const STAFF_USER = meOf({
  id: 'staff-1',
  role: 'TELLER',
  branch: 'MAIN',
  name: 'Staff User',
  branchName: 'MAIN',
  branches: [],
});

describe('AppShell customer session isolation (#1603)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let fetchUrls: string[];

  beforeEach(() => {
    vi.useFakeTimers();
    setAudience('customer');
    setLang('th');
    fetchUrls = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
      fetchUrls.push(url);

      if (url.includes('/health/ready')) {
        return new Response(JSON.stringify({ status: 'ok', version: '1.0.0' }), { status: 200 });
      }
      if (url.includes('/api/v3/views/my_profile')) {
        return new Response(JSON.stringify({ data: { displayName: 'Customer User' } }), { status: 200 });
      }
      if (url.includes('/api/v3/views/me')) {
        return new Response(
          JSON.stringify({
            version: 3,
            changed: true,
            data: STAFF_USER,
          }),
          { status: 200 },
        );
      }
      return new Response(JSON.stringify({}), { status: 200 });
    });

    vi.stubGlobal('fetch', mockFetch);

    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('mounting CustomerShell and advancing fake timers 120 seconds makes no fetch to the staff me endpoint', async () => {
    dispose = render(
      () => (
        <CustomerShell route="/app">
          <p>Customer Content</p>
        </CustomerShell>
      ),
      host,
    );

    // Initial microtasks / render
    await vi.advanceTimersByTimeAsync(0);

    // Advance fake timers by 120 seconds
    await vi.advanceTimersByTimeAsync(120_000);

    // Check that customer content rendered
    expect(host.textContent).toContain('Customer Content');

    // AccountBar must not be rendered
    expect(host.querySelector('.account-bar')).toBeNull();

    // Session-ended alert must not be rendered
    expect(host.querySelector('[role="alert"]')).toBeNull();

    // Lock fieldset must not be disabled
    const fieldset = host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement | null;
    expect(fieldset).not.toBeNull();
    expect(fieldset?.disabled).toBe(false);

    // No request to the staff me endpoint is made
    const meCalls = fetchUrls.filter((url) => url.includes('/api/v3/views/me') || url.endsWith('/me'));
    expect(meCalls).toHaveLength(0);
  });

  it('AppShell with staffSession={false} does not render AccountBar and makes no me call', async () => {
    dispose = render(
      () => (
        <AppShell title="Test" staffSession={false}>
          <p>App Content</p>
        </AppShell>
      ),
      host,
    );

    await vi.advanceTimersByTimeAsync(120_000);

    expect(host.querySelector('.account-bar')).toBeNull();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    const fieldset = host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement | null;
    expect(fieldset).not.toBeNull();
    expect(fieldset?.disabled).toBe(false);

    const meCalls = fetchUrls.filter((url) => url.includes('/api/v3/views/me') || url.endsWith('/me'));
    expect(meCalls).toHaveLength(0);
  });

  it('AppShell default (staffSession omitted) keeps the staff session and calls me poller', async () => {
    dispose = render(
      () => (
        <AppShell title="Staff Screen">
          <p>Staff Content</p>
        </AppShell>
      ),
      host,
    );

    await vi.advanceTimersByTimeAsync(0);

    const meCalls = fetchUrls.filter((url) => url.includes('/api/v3/views/me') || url.endsWith('/me'));
    expect(meCalls.length).toBeGreaterThan(0);
  });
});
