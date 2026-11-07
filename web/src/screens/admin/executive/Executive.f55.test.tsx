// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// The header's period ends are days (§16.7: a period's ends → formatBangkokDate). Harness copied from
// `Executive.time.test.tsx`. Written before the implementation by the ticket author. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import {
  makeAlertDto,
  makeBranchRowDto,
  makeDrilldownDto,
  makeExecutiveOverviewData,
  makeMetricDto,
  makeOverviewPeriod,
  makeShiftsDto,
  type ExecutiveOverviewData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { Executive } from './Executive';

const ENDPOINT = '/api/v3/views/executive_overview';
const FROM = '2026-10-06';
const TO = '2026-10-06';
const VERSIONS = { B1: 1, B2: 1 };

/** 01:30 on 7 Oct in Bangkok: the wrong day and 7 hours behind when shown raw. */
const AS_OF = '2026-10-06T18:30:00.000Z';
const SINCE = '2026-10-06T17:05:00.000Z';


function drill(view: string, filter: Record<string, string> = {}) {
  return makeDrilldownDto({ view, filter, dropped: [] });
}

function metric(key: string, value: string | null) {
  return makeMetricDto({ key, value, unavailable: null, drilldown: null });
}

function branchRow(id: string, asOf: string | null) {
  return makeBranchRowDto({
    branchId: id,
    name: `สาขา ${id}`,
    volumeThb: '3000.00',
    availableCash: metric('available_cash', '10000.00'),
    inTransit: metric('in_transit', '0.00'),
    shifts: makeShiftsDto({ open: 1, closedVerified: 0, closedUnverified: 0 }),
    asOf,
    pendingEvents: 0,
    drilldown: drill('admin_overview', { branch: id }),
  });
}

function alert(branchId: string, since: string | null) {
  return makeAlertDto({ kind: 'staleData', branchId, since, staleMinutes: 30, drilldown: null });
}

function sampleData(over: Partial<ExecutiveOverviewData> = {}) {
  return makeExecutiveOverviewData({
    period: makeOverviewPeriod({ from: FROM, to: TO }),
    kpis: [metric('buy_volume_thb', '1000.00')],
    branches: [branchRow('B1', AS_OF)],
    alerts: [alert('B1', SINCE)],
    definitions: [],
    buyReversedThb: '0.00',
    sellReversedThb: '0.00',
    ...over,
  });
}

let replyData: ExecutiveOverviewData;
let pollData: ExecutiveOverviewData | null;

function json(body: unknown, status = 200): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } }),
  );
}

function fakeFetch(input: string | URL | Request): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  if (url.pathname === '/api/v3/views/me') return json({ version: 1, changed: false });
  if (url.pathname === QUEUE_CARD_PATH) return json(queueCardUnchanged());
  if (url.pathname === ENDPOINT) {
    if (url.searchParams.has('versions')) {
      if (pollData === null) return json({ changed: false, versions: VERSIONS });
      return json({ changed: true, versions: { B1: 2 }, data: pollData });
    }
    return json({ changed: true, versions: VERSIONS, data: replyData });
  }
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function mount(from = FROM, to = TO): HTMLElement {
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from, to });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Executive branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return root;
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

beforeEach(() => {
  vi.stubEnv('TZ', 'UTC');
  replyData = sampleData();
  pollData = null;
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  setLang('th');
  await settle();
});

describe('F55: the header period is a day (dd/mm/yyyy), never the raw YYYY-MM-DD', () => {
  it('F55-EX1: a one-day period reads 06/10/2026', async () => {
    const el = mount();
    await settle();
    const crumbs = [...el.querySelectorAll('.executive-header .executive-crumb')].map((c) => text(c));
    expect(crumbs).toContain('06/10/2026');
    expect(text(el.querySelector('.executive-header'))).not.toContain('2026-10-06');
  });

  it('F55-EX2: a range reads 01/10/2026 – 06/10/2026', async () => {
    const el = mount('2026-10-01', '2026-10-06');
    await settle();
    const crumbs = [...el.querySelectorAll('.executive-header .executive-crumb')].map((c) => text(c));
    expect(crumbs).toContain('01/10/2026 – 06/10/2026');
    expect(text(el.querySelector('.executive-header'))).not.toContain('2026-10-01');
  });
});
