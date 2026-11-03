// #564 finding 3 (ADMIN at bbbfa237): the rates draft card offers "สร้างฉบับร่างใหม่" to a caller who may not draft.
// The new-draft button follows the caller's permission `RatesDraft` (server rows `rateDraft`, `rate_drafts`, `rate_draft`:
// api/crates/xc-auth/src/domain/policy/areas/rates.rs:29), never the role.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - D1: me loads without `RatesDraft` ⇒ no new-draft button is drawn (not drawn, rather than disabled: the read-only
//   pattern of #6016/#6027).
// - D2: me loads with `RatesDraft` ⇒ the button is drawn and enabled, as today.
// - D3: me fails ⇒ the permissions are unknown; the button stays as today (the server still refuses a caller without it).
// - D4: the button follows a permission re-read: me fails (button as today), then a retry loads me without `RatesDraft` ⇒ it goes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
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

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const meWith = (permissions: string[]) => () => json(200, { version: 1, changed: true, data: makeMe({ permissions }) });
const newDraftButton = () =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === 'สร้างฉบับร่างใหม่');

describe('#564 F-3: the new-draft button follows RatesDraft', () => {
  it('D1 me without RatesDraft (ADMIN: SettingsRead only): no new-draft button', async () => {
    await mountRates(createServer({ meHandler: meWith(['SettingsRead', 'UsersManage', 'BranchesManage']) }).fn);
    await vi.waitFor(() => expect(host.querySelector('[data-testid="owner-set"]')).not.toBeNull());
    expect(newDraftButton()).toBeUndefined();
  });

  it('D2 me with RatesDraft: the new-draft button is drawn and enabled', async () => {
    await mountRates(createServer({ meHandler: meWith(['SettingsRead', 'RatesDraft']) }).fn);
    await vi.waitFor(() => expect(newDraftButton()).toBeDefined());
    expect(newDraftButton()!.disabled).toBe(false);
  });

  it('D3 me fails: the permissions are unknown, so the button stays as today', async () => {
    await mountRates(createServer({ meHandler: () => json(500, { error: { code: 'internal', message: 'boom' } }) }).fn);
    await vi.waitFor(() => expect(host.querySelector('[data-permissions-unavailable]')).not.toBeNull());
    expect(newDraftButton()).toBeDefined();
  });

  it('D4 the button follows a permission re-read', async () => {
    let perms: string[] | null = null;
    await mountRates(
      createServer({
        meHandler: () => (perms ? meWith(perms)() : json(500, { error: { code: 'internal', message: 'boom' } })),
      }).fn,
    );
    await vi.waitFor(() => expect(host.querySelector('[data-permissions-unavailable]')).not.toBeNull());
    perms = ['SettingsRead'];
    host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('[data-permissions-unavailable]')).toBeNull());
    await flush();
    expect(newDraftButton(), 'retry without RatesDraft: not drawn').toBeUndefined();
  });
});
