import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RateDraftDetailData } from '../../../shared/api/generated/rate_draft';
import type { RateDraftItemDto } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatesReferenceData } from '../../../shared/api/generated/rates_reference';
import type { RatePublicationRow } from './rate-history';
import type { RatesHistoryProps } from './RatesHistory';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
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

let lastRatesHistoryProps: RatesHistoryProps | null = null;

vi.mock('./RatesHistory', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./RatesHistory')>();
  return {
    ...actual,
    RatesHistory: (props: RatesHistoryProps) => {
      lastRatesHistoryProps = props;
      return actual.RatesHistory(props);
    },
  };
});

const PUBLISHED = '/api/v3/views/published_rates';
const DRAFTS = '/api/v3/views/rate_drafts';
const DRAFT = '/api/v3/views/rate_draft';
const POLICY = '/api/v3/views/rate_policy';
const PUBLICATIONS = '/api/v3/views/rate_publications';
const REFERENCE = '/api/v3/views/rates_reference';

function sampleRows(): RatePublicationRow[] {
  return [
    {
      id: 'pub-18',
      branchId: 'MAIN',
      version: 18,
      publishedAt: '2026-10-18T10:00:00Z',
      publishedBy: 'owner-1',
      approvedRequestId: 'req-18',
      draftId: 'd-18',
      state: 'PUBLISHED',
      lines: [],
    },
    {
      id: 'pub-17',
      branchId: 'MAIN',
      version: 17,
      publishedAt: '2026-10-17T10:00:00Z',
      publishedBy: 'rates-9',
      approvedRequestId: 'req-17',
      draftId: 'd-17',
      state: 'SUPERSEDED',
      lines: [],
    },
  ];
}

function mainPublished(): PublishedRatesData {
  return {
    publication: { id: 'pub-7', version: 7, publishedAt: '2026-10-09T02:00:00+00:00' },
    sourceDate: '2026-10-08',
    stale: false,
    lines: [
      { currency: 'EUR', group: 'ALL', unitSize: '1', buyRate: '37.35520000', sellRate: '38.08440000', lineId: 'line-EUR', bands: [] },
      { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '33.25000000', sellRate: '34.10000000', lineId: 'line-USD', bands: [] },
    ],
    bandHeadings: [],
  };
}

function draftItem(id: string, status: string, version: number, createdAt: string): RateDraftItemDto {
  return {
    id,
    branchId: 'MAIN',
    status,
    version,
    createdBy: 'rates-9',
    createdAt,
    updatedAt: createdAt,
    evaluation: null,
  };
}

function detailOf(item: RateDraftItemDto): RateDraftDetailData {
  return {
    draft: { ...item },
    lines: [
      { currency: 'EUR', buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null },
      { currency: 'USD', buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null },
    ],
  };
}

function policyOf(): RatePolicyData {
  return {
    policy: {
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
    },
  };
}

function referenceOf(): RatesReferenceData {
  return {
    rows: [{ code: 'USD', date: '2026-10-08', unit: 1, buyingSight: '33.4218', buyingTransfer: '33.5008', selling: '33.8278' }],
    fetchedAt: '2026-10-09T01:30:00.000Z',
    error: null,
  };
}

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let publicationsReply: (() => Promise<Reply> | Reply) | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const s: Sent = { path: url.pathname, params: url.searchParams, method };
  sent.push(s);

  const { path } = s;
  if (path === '/api/v3/views/me') {
    return json(200, {
      version: 1,
      changed: true,
      data: { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
    });
  }
  if (path === QUEUE_CARD_PATH) return json(200, queueCardUnchanged());

  if (method === 'GET') {
    if (path === PUBLISHED) return json(200, { version: 1, changed: true, data: mainPublished() });
    if (path === DRAFTS) {
      const d1 = draftItem('d1', 'DRAFT', 1, '2026-10-09T04:00:00+00:00');
      return json(200, { version: 1, changed: true, data: { drafts: [d1] } });
    }
    if (path === DRAFT) {
      const d1 = draftItem('d1', 'DRAFT', 1, '2026-10-09T04:00:00+00:00');
      return json(200, { version: 1, changed: true, data: detailOf(d1) });
    }
    if (path === POLICY) return json(200, { version: 1, changed: true, data: policyOf() });
    if (path === REFERENCE) return json(200, { version: 1, changed: true, data: referenceOf() });
    if (path === PUBLICATIONS) {
      if (publicationsReply) {
        const reply = await publicationsReply();
        return json(reply.status, reply.body);
      }
      return json(200, sampleRows());
    }
  }

  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${path}` } });
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

function historyButton(el: HTMLElement): HTMLButtonElement {
  const buttons = Array.from(el.querySelectorAll('button'));
  const found = buttons.find((b) => b.textContent?.includes('ประวัติการปรับเรท'));
  expect(found, 'Rate history button exists').toBeDefined();
  return found!;
}

beforeEach(() => {
  sent = [];
  lastRatesHistoryProps = null;
  publicationsReply = null;
  vi.stubGlobal('fetch', fakeFetch);
  setLang('th');
  toast('');
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
  await settle();
});

describe('108-Plaza/xchange#3439: Rates history dialog wiring', () => {
  it('click history sends one GET /api/v3/views/rate_publications?branch=MAIN&limit=20', async () => {
    const m = mount();
    await settle();
    sent = [];

    historyButton(m.el).click();
    await settle();

    const pubReads = sent.filter((s) => s.path === PUBLICATIONS && s.method === 'GET');
    expect(pubReads).toHaveLength(1);
    expect(pubReads[0]!.params.get('branch')).toBe('MAIN');
    expect(pubReads[0]!.params.get('limit')).toBe('20');
  });

  it('the read answers rows: dialog .rate-history-dialog holds [data-rate-history], receives them as first, and no second read on open', async () => {
    const expectedRows = sampleRows();
    publicationsReply = () => ({ status: 200, body: expectedRows });

    const m = mount();
    await settle();
    sent = [];

    historyButton(m.el).click();
    await settle();

    const dialogEl = document.querySelector('.rate-history-dialog');
    expect(dialogEl, '.rate-history-dialog exists').not.toBeNull();
    expect(dialogEl!.querySelector('[data-rate-history]'), '[data-rate-history] inside dialog').not.toBeNull();

    expect(lastRatesHistoryProps).not.toBeNull();
    expect(lastRatesHistoryProps!.first).toEqual(expectedRows);
    expect(lastRatesHistoryProps!.branch).toBe('MAIN');

    const pubReads = sent.filter((s) => s.path === PUBLICATIONS && s.method === 'GET');
    expect(pubReads).toHaveLength(1);
  });

  it('branch switched while read in flight: no dialog opened for old branch (readGen)', async () => {
    let resolvePub: ((r: Reply) => void) | null = null;
    publicationsReply = () =>
      new Promise<Reply>((resolve) => {
        resolvePub = resolve;
      });

    const m = mount({ branch: 'MAIN' });
    await settle();

    historyButton(m.el).click();
    await Promise.resolve();

    // Switch branch while read is in flight
    m.setBranch('B9');
    await settle();

    // Now complete the pending read for MAIN
    expect(resolvePub).not.toBeNull();
    resolvePub!({ status: 200, body: sampleRows() });
    await settle();

    expect(document.querySelector('.rate-history-dialog')).toBeNull();
  });
});
