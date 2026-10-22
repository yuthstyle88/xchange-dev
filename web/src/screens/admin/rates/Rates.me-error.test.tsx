// R3-3: /admin rates — a failed me read disables the rate policy controls with no reason
// Issue #4741, design docs/design/web-silent-failures.md §7
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import {
  makeMe,
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeRateDraftsData,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  makeRatesReferenceData,
} from '../../../shared/testing/fixtures';
import { Rates } from './Rates';

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
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

function defaultPolicyData() {
  return makeRatePolicyData({
    policy: makeRatePolicyVersionDto({
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: {
        USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
      },
      maxReferenceAgeHours: 24,
      onStale: 'STOP',
      staleLimitThb: null,
      validFrom: '2026-10-08T00:00:00+00:00',
      validTo: null,
      approvedBy: 'owner-1',
      reason: 'seeded',
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    }),
  });
}

function defaultPublishedData() {
  return makePublishedRatesData({
    publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-09T02:00:00+00:00' }),
    sourceDate: '2026-10-08',
    stale: false,
    lines: [
      makePublishedLineDto({
        currency: 'USD',
        group: 'ALL',
        unitSize: '1',
        buyRate: '33.25000000',
        sellRate: '34.10000000',
        lineId: 'line-USD',
        bands: [],
      }),
    ],
    bandHeadings: [],
  });
}

function createServer(opts: {
  meHandler: () => Promise<Response> | Response;
  policyHandler?: () => Promise<Response> | Response;
}) {
  const fn = async (input: RequestInfo | URL): Promise<Response> => {
    const path = new URL(String(input), 'http://localhost').pathname;
    if (path === '/api/v3/views/me') {
      return opts.meHandler();
    }
    if (path === '/api/v3/views/rate_policy') {
      if (opts.policyHandler) return opts.policyHandler();
      await Promise.resolve();
      return new Response(JSON.stringify({ version: 1, changed: true, data: defaultPolicyData() }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (path === '/api/v3/views/published_rates') {
      return new Response(JSON.stringify({ version: 1, changed: true, data: defaultPublishedData() }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (path === '/api/v3/views/rate_drafts') {
      return new Response(JSON.stringify({ version: 1, changed: true, data: makeRateDraftsData({ drafts: [] }) }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (path === '/api/v3/views/rates_reference') {
      return new Response(
        JSON.stringify({ version: 1, changed: true, data: makeRatesReferenceData({ rows: [], fetchedAt: '', error: null }) }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
    return new Response(JSON.stringify({ error: { code: 'notFound', message: 'nf' } }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    });
  };
  return { fn };
}

async function mountRates(serverFn: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>) {
  vi.stubGlobal('fetch', vi.fn(serverFn));
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
  const [shown] = createSignal(true);
  dispose = render(() => <Rates branch={branch} period={period} shown={shown} />, host);
  await flush();
}

describe('Rates me-error (R3-3 / #4741)', () => {
  it('M1 me answers 500; rate_policy answers 200 after it: the shared [data-permissions-unavailable] line with its retry is shown; the policy save control is disabled with the reason as title', async () => {
    let resolvePolicy!: () => void;
    const policyGate = new Promise<void>((r) => {
      resolvePolicy = r;
    });

    const server = createServer({
      meHandler: () => {
        queueMicrotask(() => resolvePolicy());
        return new Response(JSON.stringify({ error: { code: 'serverError', message: 'server broke' } }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        });
      },
      policyHandler: async () => {
        await policyGate;
        return new Response(JSON.stringify({ version: 1, changed: true, data: defaultPolicyData() }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      },
    });
    await mountRates(server.fn);

    const expectedReason = permissionsUnavailableText(
      new ApiError({ status: 500, code: 'serverError', message: 'server broke' }),
    );
    const alert = host.querySelector('[data-permissions-unavailable]');
    expect(alert).not.toBeNull();
    expect(alert!.getAttribute('role')).toBe('alert');
    expect(alert!.textContent).toContain(expectedReason);

    const retry = host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
    expect(retry).not.toBeNull();
    expect(retry!.disabled).toBe(false);

    const saveBtn = host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]');
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(true);
    expect(saveBtn!.title).toBe(expectedReason);
  });

  it('M2 me fails on the network: as M1', async () => {
    const netErr = new TypeError('Failed to fetch');
    const server = createServer({
      meHandler: () => {
        throw netErr;
      },
    });
    await mountRates(server.fn);

    const expectedReason = permissionsUnavailableText(netErr);
    const alert = host.querySelector('[data-permissions-unavailable]');
    expect(alert).not.toBeNull();
    expect(alert!.getAttribute('role')).toBe('alert');
    expect(alert!.textContent).toContain(expectedReason);

    const retry = host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
    expect(retry).not.toBeNull();
    expect(retry!.disabled).toBe(false);

    const saveBtn = host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]');
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(true);
    expect(saveBtn!.title).toBe(expectedReason);
  });

  it('M3 M1, then retry, and me now answers with RatePolicySave: the line is gone; the policy save control is enabled', async () => {
    let meFails = true;
    const server = createServer({
      meHandler: () => {
        if (meFails) {
          return new Response(JSON.stringify({ error: { code: 'serverError', message: 'server broke' } }), {
            status: 500,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(
          JSON.stringify({
            version: 2,
            changed: true,
            data: makeMe({ permissions: ['RatePolicySave'] }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      },
    });
    await mountRates(server.fn);

    expect(host.querySelector('[data-permissions-unavailable]')).not.toBeNull();
    const saveBtn = host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]');
    expect(saveBtn!.disabled).toBe(true);

    meFails = false;
    const retry = host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
    retry!.click();
    await flush();

    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();
    expect(saveBtn!.disabled).toBe(false);
    expect(saveBtn!.title).toBe('');
  });

  it('M4 me loads without RatePolicySave: no permissions line; save control disabled as today (no reason title)', async () => {
    const server = createServer({
      meHandler: () =>
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: makeMe({ permissions: [] }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
    });
    await mountRates(server.fn);

    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();
    const saveBtn = host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]');
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(true);
    expect(saveBtn!.title).toBe('');
  });

  it('M5 rate_policy fails, me loads: the policy error shows as today, and no permissions line', async () => {
    const policyErr = new ApiError({ status: 500, code: 'serverError', message: 'policy broke' });
    const server = createServer({
      meHandler: () =>
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: makeMe({ permissions: ['RatePolicySave'] }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      policyHandler: () =>
        new Response(JSON.stringify({ error: { code: 'serverError', message: 'policy broke' } }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        }),
    });
    await mountRates(server.fn);

    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();
    const alerts = Array.from(host.querySelectorAll('[role="alert"]'));
    expect(alerts.some((a) => (a.textContent ?? '').includes(wordedError(policyErr)))).toBe(true);
  });
});
