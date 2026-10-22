// GME-P: /admin/pricing — a failed me read hides refresh/edit-policy silently; word it, show them disabled (G-ME follow-up)
// Issue #4726, design docs/design/web-silent-failures.md §5
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import { makeMe } from '../../../shared/testing/fixtures/common';
import { BOARD, DEFAULT_ME } from './__fixtures__/pricing-board.fixture';
import { ROUTE_RATE_REFERENCE_FETCH, VIEW_PRICING_BOARD } from './pricing-labels';
import { Pricing } from './Pricing';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

type Call = { method: string; path: string; body: unknown };

function createServer(opts: {
  meHandler: () => Promise<Response> | Response;
}) {
  const calls: Call[] = [];
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });

    if (method === 'GET' && path === `/api/v3/views/${VIEW_PRICING_BOARD}`) {
      return new Response(JSON.stringify({ version: 1, changed: true, data: BOARD }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (method === 'GET' && path === '/api/v3/views/me') {
      return opts.meHandler();
    }
    if (method === 'POST' && path === ROUTE_RATE_REFERENCE_FETCH) {
      return new Response(JSON.stringify({}), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: { code: 'notFound', message: 'nf' } }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    });
  };
  return { fn, calls };
}

async function mountPricing(serverFn: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>) {
  vi.stubGlobal('fetch', vi.fn(serverFn));
  dispose = render(
    () => <Pricing branch={() => 'MAIN'} period={() => ({ from: '', to: '' })} shown={() => true} />,
    host,
  );
  await flush();
}

describe('Pricing me-error (G-ME / #4726)', () => {
  it('P1 me answers 500: the shared role="alert" line with retry; refresh and edit-policy controls present but disabled, each with reason as title', async () => {
    const server500 = createServer({
      meHandler: () =>
        new Response(JSON.stringify({ error: { code: 'serverError', message: 'server broke' } }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        }),
    });
    await mountPricing(server500.fn);

    const alert = host.querySelector('[data-permissions-unavailable]');
    expect(alert).not.toBeNull();
    expect(alert!.getAttribute('role')).toBe('alert');
    const expectedReason = permissionsUnavailableText(
      new ApiError({ status: 500, code: 'serverError', message: 'server broke' }),
    );
    expect(alert!.textContent).toContain(expectedReason);

    const retry = host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
    expect(retry).not.toBeNull();
    expect(retry!.disabled).toBe(false);

    // Refresh controls present but disabled with reason as title
    const refreshBtns = host.querySelectorAll<HTMLButtonElement>('.xc-source-refresh-btn');
    expect(refreshBtns.length).toBeGreaterThan(0);
    for (const btn of refreshBtns) {
      expect(btn.disabled).toBe(true);
      expect(btn.title).toBe(expectedReason);
    }

    // Open a row to inspect edit-policy controls
    const codeBtn = host.querySelector<HTMLButtonElement>('.pricing-btn-code');
    expect(codeBtn).not.toBeNull();
    codeBtn!.click();
    await flush();

    const saveBtn = host.querySelector<HTMLButtonElement>('.pricing-policy-btn-save');
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(true);
    expect(saveBtn!.title).toBe(expectedReason);

    const clearBtn = host.querySelector<HTMLButtonElement>('.pricing-policy-btn-clear');
    expect(clearBtn).not.toBeNull();
    expect(clearBtn!.disabled).toBe(true);
    expect(clearBtn!.title).toBe(expectedReason);
  });

  it('P2 me fails on the network: as P1', async () => {
    const netErr = new TypeError('Failed to fetch');
    const serverNet = createServer({
      meHandler: () => {
        throw netErr;
      },
    });
    await mountPricing(serverNet.fn);

    const alert = host.querySelector('[data-permissions-unavailable]');
    expect(alert).not.toBeNull();
    expect(alert!.getAttribute('role')).toBe('alert');
    const expectedReason = permissionsUnavailableText(netErr);
    expect(alert!.textContent).toContain(expectedReason);

    const refreshBtns = host.querySelectorAll<HTMLButtonElement>('.xc-source-refresh-btn');
    expect(refreshBtns.length).toBeGreaterThan(0);
    for (const btn of refreshBtns) {
      expect(btn.disabled).toBe(true);
      expect(btn.title).toBe(expectedReason);
    }

    const codeBtn = host.querySelector<HTMLButtonElement>('.pricing-btn-code');
    codeBtn!.click();
    await flush();

    const saveBtn = host.querySelector<HTMLButtonElement>('.pricing-policy-btn-save');
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(true);
    expect(saveBtn!.title).toBe(expectedReason);
  });

  it('P3 P1, then retry, and me now answers with MarketRatesRefresh: the line is gone and refresh is enabled', async () => {
    let meStatus = 500;
    const server = createServer({
      meHandler: () => {
        if (meStatus === 500) {
          return new Response(JSON.stringify({ error: { code: 'serverError', message: 'broke' } }), {
            status: 500,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(JSON.stringify({ version: 2, changed: true, data: DEFAULT_ME }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      },
    });

    await mountPricing(server.fn);

    expect(host.querySelector('[data-permissions-unavailable]')).not.toBeNull();
    const initialRefresh = host.querySelector<HTMLButtonElement>('.xc-source-refresh-btn');
    expect(initialRefresh?.disabled).toBe(true);

    // Switch me answer to good (holding MarketRatesRefresh)
    meStatus = 200;
    const retry = host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
    expect(retry).not.toBeNull();
    retry!.click();
    await flush();

    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();

    const activeRefreshBtns = host.querySelectorAll<HTMLButtonElement>('.xc-source-refresh-btn');
    expect(activeRefreshBtns.length).toBeGreaterThan(0);
    for (const btn of activeRefreshBtns) {
      expect(btn.disabled).toBe(false);
      expect(btn.title).toBe('');
    }
  });

  it('P4 me loads without MarketRatesRefresh: refresh is hidden, as today', async () => {
    const noRefreshMe = makeMe({
      branch: 'MAIN',
      roles: [],
      permissions: ['ReferenceRatesRead'],
    });
    const server = createServer({
      meHandler: () =>
        new Response(JSON.stringify({ version: 1, changed: true, data: noRefreshMe }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    });
    await mountPricing(server.fn);

    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();
    expect(host.querySelectorAll('.xc-source-refresh-btn')).toHaveLength(0);
  });

  it('P5 P1: press the disabled refresh: no pricingRefresh request (or whatever command refresh sends)', async () => {
    const server500 = createServer({
      meHandler: () =>
        new Response(JSON.stringify({ error: { code: 'serverError', message: 'broke' } }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        }),
    });
    await mountPricing(server500.fn);

    const refreshBtn = host.querySelector<HTMLButtonElement>('.xc-source-refresh-btn');
    expect(refreshBtn).not.toBeNull();
    expect(refreshBtn!.disabled).toBe(true);

    const postCallsBefore = server500.calls.filter((c) => c.method === 'POST');
    refreshBtn!.click();
    await flush();

    const postCallsAfter = server500.calls.filter((c) => c.method === 'POST');
    expect(postCallsAfter).toHaveLength(postCallsBefore.length);
  });
});
