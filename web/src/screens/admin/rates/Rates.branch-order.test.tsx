import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeRateDraftDetailData,
  makeRateDraftDetailItemDto,
  makeRateDraftItemDto,
  makeRateDraftLineDto,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  type PublishedLineDto,
  type RateDraftItemDto,
  type RateDraftLineDto,
  type RatePolicyData,
} from '../../../shared/testing/fixtures';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!.menu;
}

const PUBLISHED = '/api/v3/views/published_rates';
const DRAFTS = '/api/v3/views/rate_drafts';
const DRAFT = '/api/v3/views/rate_draft';
const POLICY = '/api/v3/views/rate_policy';
const PUBLICATIONS = '/api/v3/views/rate_publications';
const REFERENCE = '/api/v3/views/rates_reference';

function line(currency: string, unitSize: string, buy: string | null, sell: string | null) {
  return makePublishedLineDto({ currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}` });
}

function publishedOf(version: number | null, lines: PublishedLineDto[]) {
  return makePublishedRatesData({
    publication:
      version === null
        ? null
        : makePublishedPublicationDto({ id: `pub-${version}`, version, publishedAt: '2026-10-09T02:00:00+00:00' }),
    sourceDate: '2026-10-08',
    stale: false,
    lines,
  });
}

function draftItem(id: string, branchId: string, status: string, version: number) {
  return makeRateDraftItemDto({
    id,
    branchId,
    status,
    version,
    createdBy: 'rates-9',
    createdAt: '2026-10-09T04:00:00+00:00',
    updatedAt: '2026-10-09T04:00:00+00:00',
    evaluation: null,
  });
}

function auto(currency: string) {
  return makeRateDraftLineDto({ currency, buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null });
}

function detailOf(item: RateDraftItemDto, lines: RateDraftLineDto[]) {
  return makeRateDraftDetailData({ draft: makeRateDraftDetailItemDto({ ...item }), lines });
}

const GUARDS = {
  USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
};

function policyOf(overrides: Partial<NonNullable<RatePolicyData['policy']>> = {}) {
  return makeRatePolicyData({
    policy: makeRatePolicyVersionDto({
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: structuredClone(GUARDS),
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
      ...overrides,
    }),
  });
}

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}

type Reply = { status: number; body: unknown };

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

let sent: Sent[];
let publishedHandler: ((s: Sent) => Promise<Reply> | Reply) | null;
let draftsHandler: ((s: Sent) => Promise<Reply> | Reply) | null;
let draftDetailHandler: ((s: Sent) => Promise<Reply> | Reply) | null;
let policyHandler: ((s: Sent) => Promise<Reply> | Reply) | null;
let commandHandler: ((s: Sent) => Promise<Reply> | Reply | null) | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

async function answer(s: Sent): Promise<Reply> {
  const { path, method } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: {
          kind: 'owner',
          id: 'owner-1',
          role: 'OWNER',
          branch: 'MAIN',
          name: 'Owner',
          username: 'owner',
          permissions: ['OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'SystemSettings'],
        },
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };

  if (method === 'GET') {
    if (path === PUBLISHED) {
      if (publishedHandler) return publishedHandler(s);
      return {
        status: 200,
        body: { version: 1, changed: true, data: publishedOf(7, [line('USD', '1', '33.25000000', '34.10000000')]) },
      };
    }
    if (path === DRAFTS) {
      if (draftsHandler) return draftsHandler(s);
      const d1 = draftItem('d1', s.params.get('branch') ?? 'MAIN', 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [d1] } } };
    }
    if (path === DRAFT) {
      if (draftDetailHandler) return draftDetailHandler(s);
      const d1 = draftItem('d1', s.params.get('branch') ?? 'MAIN', 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: detailOf(d1, [auto('USD')]) } };
    }
    if (path === POLICY) {
      if (policyHandler) return policyHandler(s);
      return { status: 200, body: { version: 1, changed: true, data: policyOf() } };
    }
    if (path === REFERENCE) {
      return {
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: { rows: [], fetchedAt: '2026-10-09T01:30:00.000Z', error: null },
        },
      };
    }
    if (path === PUBLICATIONS) {
      return { status: 200, body: { version: 1, changed: true, data: { publications: [] } } };
    }
    return refusal(500, 'serverError', `unexpected read ${path}`);
  }

  if (commandHandler) {
    const custom = await commandHandler(s);
    if (custom) return custom;
  }

  if (path === '/api/rate-references/fetch' && method === 'POST') {
    return { status: 200, body: { batchId: 'batch-1' } };
  }
  if (path === '/api/rate-policies' && method === 'POST') {
    return { status: 200, body: { id: 'pol-saved', version: 2 } };
  }
  if (path === '/api/rate-drafts' && method === 'POST') {
    return { status: 200, body: { id: 'd-new-1', version: 1, status: 'DRAFT' } };
  }
  if (path.endsWith('/submit') && method === 'POST') {
    return { status: 200, body: { requestId: 'req-1', status: 'PENDING_APPROVAL' } };
  }
  if (path.endsWith('/publish') && method === 'POST') {
    return { status: 200, body: { id: 'pub-next', version: 13 } };
  }
  if (path.startsWith('/api/rate-drafts/') && method === 'PUT') {
    return { status: 200, body: { id: 'd1', version: 2, status: 'DRAFT' } };
  }

  return refusal(500, 'serverError', `unexpected ${method} ${path}`);
}

function headerOf(init: RequestInit | undefined, name: string): string | null {
  const h = init?.headers;
  if (!h) return null;
  if (h instanceof Headers) return h.get(name);
  if (Array.isArray(h)) return h.find(([k]) => k.toLowerCase() === name)?.[1] ?? null;
  const rec = h as Record<string, string>;
  const k = Object.keys(rec).find((x) => x.toLowerCase() === name);
  return k === undefined ? null : rec[k]!;
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const s: Sent = { path: url.pathname, params: url.searchParams, method, key: headerOf(init, 'idempotency-key'), body };
  sent.push(s);
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(init: { branch?: string } = {}): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  return { el, setBranch };
}

beforeEach(() => {
  sent = [];
  publishedHandler = null;
  draftsHandler = null;
  draftDetailHandler = null;
  policyHandler = null;
  commandHandler = null;
  vi.stubGlobal('fetch', fakeFetch);
  setLang('th');
  toast('');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
});

describe('108-Plaza/xchange#2308: Rates branch-switch ordering', () => {
  it("Row 1: Switch MAIN to B9; MAIN's rate_policy answers last -> Form shows B9's policy; Save sends B9's values", async () => {
    const mainPolicyDef = deferred<Reply>();
    policyHandler = (s) => {
      const b = s.params.get('branch');
      if (b === 'MAIN') return mainPolicyDef.promise;
      return {
        status: 200,
        body: {
          version: 2,
          changed: true,
          data: policyOf({
            id: 'pol-b9',
            branchId: 'B9',
            version: 2,
            buyAdjustPct: '-0.1500',
            sellAdjustPct: '0.2500',
            sharpMoveAlertPct: '2.75',
            valueTiersOn: false,
            valueTiers: [],
          }),
        },
      };
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    m.setBranch('B9');
    await settle();

    // Now MAIN's older rate_policy answers after B9's has already loaded
    mainPolicyDef.resolve({
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: policyOf({
          id: 'pol-main',
          branchId: 'MAIN',
          version: 1,
          buyAdjustPct: '-0.8500',
          sellAdjustPct: '0.9500',
          sharpMoveAlertPct: '9.50',
          valueTiersOn: true,
          valueTiers: [
            { tierNo: 1, upperInclusiveThb: '10000', buyMarginPct: '0.10', sellMarginPct: '0.10' },
            { tierNo: 2, upperInclusiveThb: '50000', buyMarginPct: '0.20', sellMarginPct: '0.20' },
            { tierNo: 3, upperInclusiveThb: '100000', buyMarginPct: '0.30', sellMarginPct: '0.30' },
            { tierNo: 4, upperInclusiveThb: null, buyMarginPct: '0.40', sellMarginPct: '0.40' },
          ],
        }),
      },
    });
    await settle();

    const sharpInput = m.el.querySelector('[data-testid="sharp-move-threshold"]') as HTMLInputElement | null;
    const tiersSwitch = m.el.querySelector('[data-testid="value-tiers-switch"]') as HTMLInputElement | null;
    expect(sharpInput?.value).toBe('2.75');
    expect(tiersSwitch?.checked).toBe(false);

    const saveBtn = m.el.querySelector('[data-testid="save-policy"]') as HTMLButtonElement | null;
    expect(saveBtn).not.toBeNull();
    expect(saveBtn!.disabled).toBe(false);
    saveBtn!.click();
    await settle();

    const saves = sent.filter((s) => s.path === '/api/rate-policies' && s.method === 'POST');
    expect(saves).toHaveLength(1);
    expect(saves[0]!.body).toMatchObject({
      branch: 'B9',
      buyAdjustPct: '-0.1500',
      sellAdjustPct: '0.2500',
      sharpMoveAlertPct: '2.75',
      valueTiersOn: false,
      valueTiers: [],
    });
  });

  it("Row 2: Switch MAIN to B9; MAIN's published_rates and rate_drafts/rate_draft answer last -> B9's lines and draft shown", async () => {
    const b0DraftsDef = deferred<Reply>();
    const mainPubDef = deferred<Reply>();
    const mainDetailDef = deferred<Reply>();

    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainPubDef.promise;
      return {
        status: 200,
        body: {
          version: 12,
          changed: true,
          data: publishedOf(12, [line('JPY', '100', '22.10000000', '22.50000000')]),
        },
      };
    };

    draftsHandler = (s) => {
      const b = s.params.get('branch');
      if (b === 'B0') return b0DraftsDef.promise;
      if (b === 'MAIN') {
        const dMain = draftItem('d-main', 'MAIN', 'APPROVED', 1);
        return { status: 200, body: { version: 1, changed: true, data: { drafts: [dMain] } } };
      }
      const dB9 = draftItem('d-b9', 'B9', 'APPROVED', 3);
      return { status: 200, body: { version: 3, changed: true, data: { drafts: [dB9] } } };
    };

    draftDetailHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainDetailDef.promise;
      const dB9 = draftItem('d-b9', 'B9', 'APPROVED', 3);
      return { status: 200, body: { version: 3, changed: true, data: detailOf(dB9, [auto('JPY')]) } };
    };

    const m = mount({ branch: 'B0' });
    await settle();

    // Switch to MAIN: rate_drafts resolves immediately so inner rate_draft is now awaiting
    m.setBranch('MAIN');
    await settle();

    // Switch to B9: B9's published_rates, rate_drafts, and rate_draft resolve
    m.setBranch('B9');
    await settle();

    expect(m.el.querySelector('[data-published-line="JPY"]')).not.toBeNull();
    expect(m.el.querySelector('[data-draft-line="JPY"]')).not.toBeNull();

    // Now B0's outer rate_drafts (empty), MAIN's inner rate_draft (EUR), and MAIN's published_rates (EUR) resolve late
    const dMain = draftItem('d-main', 'MAIN', 'APPROVED', 1);
    b0DraftsDef.resolve({
      status: 200,
      body: { version: 1, changed: true, data: { drafts: [] } },
    });
    mainPubDef.resolve({
      status: 200,
      body: {
        version: 7,
        changed: true,
        data: publishedOf(7, [line('EUR', '1', '37.35520000', '38.08440000')]),
      },
    });
    mainDetailDef.resolve({
      status: 200,
      body: { version: 1, changed: true, data: detailOf(dMain, [auto('EUR')]) },
    });
    await settle();

    expect(m.el.querySelector('[data-published-line="JPY"]')).not.toBeNull();
    expect(m.el.querySelector('[data-published-line="EUR"]')).toBeNull();
    expect(m.el.querySelector('[data-draft-line="JPY"]')).not.toBeNull();
    expect(m.el.querySelector('[data-draft-line="EUR"]')).toBeNull();

    const publishBtn = m.el.querySelector('.btn-publish') as HTMLButtonElement | null;
    expect(publishBtn).not.toBeNull();
    publishBtn!.click();
    await settle();

    const publishes = sent.filter((s) => s.path.endsWith('/publish') && s.method === 'POST');
    expect(publishes).toHaveLength(1);
    expect(publishes[0]!.path).toBe('/api/rate-drafts/d-b9/publish');
    expect(publishes[0]!.body).toEqual({ branch: 'B9', expectedVersion: 12 });
  });

  it('Row 3: A failed old-branch read after the switch -> No error on the new branch', async () => {
    const mainPubDef = deferred<Reply>();
    const mainDraftsDef = deferred<Reply>();
    const mainPolicyDef = deferred<Reply>();

    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainPubDef.promise;
      return {
        status: 200,
        body: { version: 2, changed: true, data: publishedOf(2, [line('USD', '1', '33.00000000', '34.00000000')]) },
      };
    };
    draftsHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainDraftsDef.promise;
      const dB9 = draftItem('d-b9', 'B9', 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [dB9] } } };
    };
    policyHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainPolicyDef.promise;
      return { status: 200, body: { version: 2, changed: true, data: policyOf({ branchId: 'B9' }) } };
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    m.setBranch('B9');
    await settle();

    mainPubDef.resolve(refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    mainDraftsDef.resolve(refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    mainPolicyDef.resolve(refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    await settle();

    const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
    expect(alerts).toHaveLength(0);
    expect(m.el.querySelector('[data-published-line="USD"]')).not.toBeNull();
  });

  it("Row 4: New branch's read fails -> Previous branch's rates stay cleared even when old-branch read answers late", async () => {
    const mainPubDef = deferred<Reply>();
    publishedHandler = (s) => {
      const b = s.params.get('branch');
      if (b === 'B0') {
        return {
          status: 200,
          body: { version: 1, changed: true, data: publishedOf(1, [line('EUR', '1', '37.35520000', '38.08440000')]) },
        };
      }
      if (b === 'MAIN') return mainPubDef.promise;
      return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    };

    const m = mount({ branch: 'B0' });
    await settle();
    expect(m.el.querySelector('[data-published-line="EUR"]')).not.toBeNull();

    // Switch to MAIN (in-flight), then to B9 (which fails)
    m.setBranch('MAIN');
    await settle();
    m.setBranch('B9');
    await settle();

    // Now MAIN's read answers late with EUR lines
    mainPubDef.resolve({
      status: 200,
      body: { version: 1, changed: true, data: publishedOf(1, [line('EUR', '1', '37.35520000', '38.08440000')]) },
    });
    await settle();

    const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
    expect(alerts.some((a) => a.textContent?.includes(expected))).toBe(true);
    expect(m.el.querySelector('[data-published-line="EUR"]')).toBeNull();
  });

  it('Row 5: Policy failed to load on new branch -> No POST of policy even if old-branch policy answers late', async () => {
    const mainPolicyDef = deferred<Reply>();
    policyHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') return mainPolicyDef.promise;
      return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    m.setBranch('B9');
    await settle();

    // MAIN's policy resolves late after B9's policy failed
    mainPolicyDef.resolve({
      status: 200,
      body: { version: 1, changed: true, data: policyOf({ branchId: 'MAIN', buyAdjustPct: '-0.5000' }) },
    });
    await settle();

    const saveBtn = m.el.querySelector('[data-testid="save-policy"]') as HTMLButtonElement | null;
    const adjustAllBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
    expect(saveBtn?.disabled).toBe(true);
    expect(adjustAllBtn?.disabled).toBe(true);

    saveBtn?.click();
    adjustAllBtn?.click();
    await settle();

    const policyWrites = sent.filter((s) => s.path === '/api/rate-policies' && s.method === 'POST');
    expect(policyWrites).toHaveLength(0);
  });

  it('Edge case: A to B to A with the first A answer last -> the latest A read wins', async () => {
    let mainPolicyCalls = 0;
    let mainPubCalls = 0;
    const firstMainPolicy = deferred<Reply>();
    const firstMainPub = deferred<Reply>();

    policyHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        mainPolicyCalls++;
        if (mainPolicyCalls === 1) return firstMainPolicy.promise;
        return {
          status: 200,
          body: {
            version: 3,
            changed: true,
            data: policyOf({
              id: 'pol-main-latest',
              branchId: 'MAIN',
              version: 3,
              buyAdjustPct: '-0.2500',
              sellAdjustPct: '0.3500',
              sharpMoveAlertPct: '3.25',
            }),
          },
        };
      }
      return {
        status: 200,
        body: {
          version: 2,
          changed: true,
          data: policyOf({ id: 'pol-b9', branchId: 'B9', sharpMoveAlertPct: '1.00' }),
        },
      };
    };

    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        mainPubCalls++;
        if (mainPubCalls === 1) return firstMainPub.promise;
        return {
          status: 200,
          body: {
            version: 9,
            changed: true,
            data: publishedOf(9, [line('GBP', '1', '42.00000000', '43.00000000')]),
          },
        };
      }
      return {
        status: 200,
        body: {
          version: 8,
          changed: true,
          data: publishedOf(8, [line('JPY', '100', '22.00000000', '23.00000000')]),
        },
      };
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    m.setBranch('B9');
    await settle();

    m.setBranch('MAIN');
    await settle();

    // Now the first MAIN request answers last with stale values
    firstMainPolicy.resolve({
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: policyOf({
          id: 'pol-main-stale',
          branchId: 'MAIN',
          version: 1,
          buyAdjustPct: '-0.9900',
          sellAdjustPct: '0.9900',
          sharpMoveAlertPct: '9.99',
        }),
      },
    });
    firstMainPub.resolve({
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: publishedOf(1, [line('EUR', '1', '37.00000000', '38.00000000')]),
      },
    });
    await settle();

    const sharpInput = m.el.querySelector('[data-testid="sharp-move-threshold"]') as HTMLInputElement | null;
    expect(sharpInput?.value).toBe('3.25');
    expect(m.el.querySelector('[data-published-line="GBP"]')).not.toBeNull();
    expect(m.el.querySelector('[data-published-line="EUR"]')).toBeNull();
  });

  it("Write-then-switch (publish): publish sent on MAIN answering after switch to B9 -> B9 shows nothing of MAIN's", async () => {
    const publishDef = deferred<Reply>();
    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        return {
          status: 200,
          body: { version: 7, changed: true, data: publishedOf(7, [line('EUR', '1', '37.35520000', '38.08440000')]) },
        };
      }
      return {
        status: 200,
        body: { version: 12, changed: true, data: publishedOf(12, [line('JPY', '100', '22.10000000', '22.50000000')]) },
      };
    };
    draftsHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        const dMain = draftItem('d-main', 'MAIN', 'APPROVED', 1);
        return { status: 200, body: { version: 1, changed: true, data: { drafts: [dMain] } } };
      }
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [] } } };
    };
    draftDetailHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        const dMain = draftItem('d-main', 'MAIN', 'APPROVED', 1);
        return { status: 200, body: { version: 1, changed: true, data: detailOf(dMain, [auto('EUR')]) } };
      }
      return { status: 200, body: { version: 1, changed: true, data: { draft: null, lines: [] } } };
    };
    commandHandler = (s) => {
      if (s.path === '/api/rate-drafts/d-main/publish' && s.method === 'POST') {
        return publishDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const publishBtn = m.el.querySelector('.btn-publish') as HTMLButtonElement | null;
    expect(publishBtn).not.toBeNull();
    publishBtn!.click();
    await settle();

    m.setBranch('B9');
    await settle();

    publishDef.resolve({ status: 200, body: { id: 'pub-8', version: 8 } });
    await settle();

    expect(m.el.querySelector('[data-published-line="JPY"]')).not.toBeNull();
    expect(m.el.querySelector('[data-published-line="EUR"]')).toBeNull();
    expect(m.el.querySelector('[data-draft-line="EUR"]')).toBeNull();
  });

  it("Write-then-switch (new draft): new draft sent on MAIN answering after switch to B9 -> B9 shows nothing of MAIN's", async () => {
    const createDraftDef = deferred<Reply>();
    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        return {
          status: 200,
          body: { version: 7, changed: true, data: publishedOf(7, [line('EUR', '1', '37.35520000', '38.08440000')]) },
        };
      }
      return {
        status: 200,
        body: { version: 12, changed: true, data: publishedOf(12, [line('JPY', '100', '22.10000000', '22.50000000')]) },
      };
    };
    draftsHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        const dMain = draftItem('d-main-new', 'MAIN', 'DRAFT', 1);
        return { status: 200, body: { version: 1, changed: true, data: { drafts: [dMain] } } };
      }
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [] } } };
    };
    draftDetailHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        const dMain = draftItem('d-main-new', 'MAIN', 'DRAFT', 1);
        return { status: 200, body: { version: 1, changed: true, data: detailOf(dMain, [auto('EUR')]) } };
      }
      return { status: 200, body: { version: 1, changed: true, data: { draft: null, lines: [] } } };
    };
    commandHandler = (s) => {
      if (s.path === '/api/rate-drafts' && s.method === 'POST') {
        return createDraftDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const newDraftBtn = m.el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
    expect(newDraftBtn).not.toBeNull();
    newDraftBtn!.click();
    await settle();

    m.setBranch('B9');
    await settle();

    createDraftDef.resolve({ status: 200, body: { id: 'd-main-new', version: 1, status: 'DRAFT' } });
    await settle();

    expect(m.el.querySelector('[data-draft-line="EUR"]')).toBeNull();
    expect(m.el.querySelector('.draft-status-label')).toBeNull();
  });

  it("Write-then-switch (submit): submit sent on MAIN answering after switch to B9 -> B9 shows nothing of MAIN's", async () => {
    const submitDef = deferred<Reply>();
    draftsHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [d] } } };
    };
    draftDetailHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return {
        status: 200,
        body: { version: 1, changed: true, data: detailOf(d, [auto(b === 'MAIN' ? 'EUR' : 'JPY')]) },
      };
    };
    commandHandler = (s) => {
      if (s.path === '/api/rate-drafts/d-main/submit' && s.method === 'POST') {
        return submitDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const submitBtn = m.el.querySelector('.btn-submit') as HTMLButtonElement | null;
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();
    await settle();

    m.setBranch('B9');
    await settle();

    const beforeStatus = m.el.querySelector('.draft-status-label')?.textContent;
    expect(beforeStatus).toBeTruthy();

    submitDef.resolve({ status: 200, body: { requestId: 'req-1', status: 'PENDING_APPROVAL' } });
    await settle();

    expect(m.el.querySelector('.draft-status-label')?.textContent).toBe(beforeStatus);
    expect(m.el.querySelector('[data-draft-line="JPY"]')).not.toBeNull();
  });

  it("Write-then-switch (adjust-all): adjust-all sent on MAIN answering after switch to B9 -> B9 shows nothing of MAIN's", async () => {
    const saveDraftDef = deferred<Reply>();
    draftsHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [d] } } };
    };
    draftDetailHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return {
        status: 200,
        body: { version: 1, changed: true, data: detailOf(d, [auto(b === 'MAIN' ? 'EUR' : 'JPY')]) },
      };
    };
    commandHandler = (s) => {
      if (s.path === '/api/rate-drafts/d-main' && s.method === 'PUT') {
        return saveDraftDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const adjustAllBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
    expect(adjustAllBtn).not.toBeNull();
    adjustAllBtn!.click();
    await settle();

    m.setBranch('B9');
    await settle();

    expect(m.el.querySelector('[data-draft-line="JPY"]')).not.toBeNull();

    saveDraftDef.resolve({ status: 200, body: { id: 'd-main', version: 2, status: 'DRAFT' } });
    await settle();

    expect(m.el.querySelector('[data-draft-line="JPY"]')).not.toBeNull();
    expect(m.el.querySelector('[data-draft-line="EUR"]')).toBeNull();
  });

  it("Write-then-switch (new draft fetch-reference): switch during fetch-reference POST builds createDraft from MAIN's lines, not B9's", async () => {
    const fetchRefDef = deferred<Reply>();
    publishedHandler = (s) => {
      if (s.params.get('branch') === 'MAIN') {
        return {
          status: 200,
          body: { version: 7, changed: true, data: publishedOf(7, [line('EUR', '1', '37.35520000', '38.08440000')]) },
        };
      }
      return {
        status: 200,
        body: { version: 12, changed: true, data: publishedOf(12, [line('JPY', '100', '22.10000000', '22.50000000')]) },
      };
    };
    draftsHandler = () => ({ status: 200, body: { version: 1, changed: true, data: { drafts: [] } } });
    commandHandler = (s) => {
      if (s.path === '/api/rate-references/fetch' && s.method === 'POST') {
        return fetchRefDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const newDraftBtn = m.el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
    expect(newDraftBtn).not.toBeNull();
    newDraftBtn!.click();
    await settle();

    // Switch to B9 while fetch-reference POST is still in-flight; B9 loads JPY published rates
    m.setBranch('B9');
    await settle();
    expect(m.el.querySelector('[data-published-line="JPY"]')).not.toBeNull();

    fetchRefDef.resolve({ status: 200, body: { batchId: 'batch-1' } });
    await settle();

    const createCalls = sent.filter((s) => s.path === '/api/rate-drafts' && s.method === 'POST');
    expect(createCalls).toHaveLength(1);
    expect(createCalls[0]!.body).toEqual({
      branch: 'MAIN',
      lines: [
        {
          currency: 'EUR',
          buyMode: 'AUTO',
          sellMode: 'AUTO',
          manualBuy: null,
          manualSell: null,
          reason: null,
        },
      ],
    });
  });

  it("Write-then-switch (simulate): simulate sent on MAIN answering after switch to B9 -> B9 does not show MAIN's simulate result", async () => {
    const simulateDef = deferred<Reply>();
    draftsHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [d] } } };
    };
    draftDetailHandler = (s) => {
      const b = s.params.get('branch') ?? 'MAIN';
      const d = draftItem(b === 'MAIN' ? 'd-main' : 'd-b9', b, 'DRAFT', 1);
      return {
        status: 200,
        body: { version: 1, changed: true, data: detailOf(d, [auto(b === 'MAIN' ? 'EUR' : 'JPY')]) },
      };
    };
    commandHandler = (s) => {
      if (s.path === '/api/rate-drafts/d-main/simulate' && s.method === 'POST') {
        return simulateDef.promise;
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    const simBtn = m.el.querySelector('.btn-simulate') as HTMLButtonElement | null;
    expect(simBtn).not.toBeNull();
    simBtn!.click();
    await settle();

    m.setBranch('B9');
    await settle();

    simulateDef.resolve({
      status: 200,
      body: { id: 'd-main', isInBound: true, outOfBound: [{ currency: 'JPY', side: 'BUY' }] },
    });
    await settle();

    expect(m.el.querySelector('.in-bound-status')).toBeNull();
    expect(m.el.querySelector('.out-of-bound')).toBeNull();
  });

  it('Write-then-switch (refused write toast): writes refused on MAIN after switch to B9 -> no toast on B9', async () => {
    const fetchRefDef = deferred<Reply>();
    const simulateDef = deferred<Reply>();
    const submitDef = deferred<Reply>();
    const publishDef = deferred<Reply>();
    const adjustDraftDef = deferred<Reply>();
    const savePolicyDef = deferred<Reply>();

    commandHandler = (s) => {
      if (s.path === '/api/rate-references/fetch' && s.method === 'POST') return fetchRefDef.promise;
      if (s.path.endsWith('/simulate') && s.method === 'POST') return simulateDef.promise;
      if (s.path.endsWith('/submit') && s.method === 'POST') return submitDef.promise;
      if (s.path.endsWith('/publish') && s.method === 'POST') return publishDef.promise;
      if (s.path.startsWith('/api/rate-drafts/') && s.method === 'PUT') return adjustDraftDef.promise;
      if (s.path === '/api/rate-policies' && s.method === 'POST') {
        if (s.body && 'valueTiersOn' in s.body) return savePolicyDef.promise;
        return { status: 200, body: { id: 'pol-adj', version: 2 } };
      }
      return null;
    };

    const m = mount({ branch: 'MAIN' });
    await settle();

    (m.el.querySelector('.btn-new-draft') as HTMLButtonElement).click();
    (m.el.querySelector('.btn-simulate') as HTMLButtonElement).click();
    (m.el.querySelector('.btn-submit') as HTMLButtonElement).click();
    (m.el.querySelector('.btn-publish') as HTMLButtonElement).click();
    (m.el.querySelector('.btn-adjust-all') as HTMLButtonElement).click();
    (m.el.querySelector('[data-testid="save-policy"]') as HTMLButtonElement).click();
    await settle();

    m.setBranch('B9');
    await settle();

    for (const def of [fetchRefDef, simulateDef, submitDef, publishDef, adjustDraftDef, savePolicyDef]) {
      def.resolve(refusal(409, 'staleVersion', 'ข้อมูลถูกเปลี่ยนแล้ว'));
      await settle();
      const t = document.querySelector('.toast');
      const toastShown = Boolean(t && !t.hasAttribute('hidden') && (t.textContent?.trim() ?? '') !== '');
      expect(toastShown).toBe(false);
    }
  });
});
