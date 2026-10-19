// #2497: A01's "as of" and alert "since" times are shown as Bangkok date + time in the screen's locale,
// not as the raw UTC string the server sends. The `datetime` attribute stays the server's string.
// Setup copied from `Executive.reversals.test.tsx`.

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
import * as i18n from './i18n';

const ENDPOINT = '/api/v3/views/executive_overview';
const FROM = '2026-10-06';
const TO = '2026-10-06';
const VERSIONS = { B1: 1, B2: 1 };

/** 01:30 on 7 Oct in Bangkok: the wrong day and 7 hours behind when shown raw. */
const AS_OF = '2026-10-06T18:30:00.000Z';
const SINCE = '2026-10-06T17:05:00.000Z';

const AS_OF_TH = '7/10/2569 01:30:00';
const SINCE_TH = '7/10/2569 00:05:00';
const AS_OF_EN = '07/10/2026, 01:30:00';
const SINCE_EN = '07/10/2026, 00:05:00';

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
  return makeAlertDto({ kind: 'staleData', branchId, since, detail: 'detail', drilldown: null });
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

function mount(): HTMLElement {
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: FROM, to: TO });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Executive branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return root;
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function headerTime(el: HTMLElement): HTMLTimeElement {
  const found = el.querySelector<HTMLTimeElement>('.executive-header time');
  expect(found, 'the header has a <time>').not.toBeNull();
  return found!;
}

function rowTime(el: HTMLElement, id: string): HTMLTimeElement {
  const found = el.querySelector<HTMLTimeElement>(`[data-branch="${id}"] time`);
  expect(found, `row ${id} has a <time>`).not.toBeNull();
  return found!;
}

function alertTime(el: HTMLElement): HTMLTimeElement {
  const found = el.querySelector<HTMLTimeElement>('[data-alert-kind] time');
  expect(found, 'the alert has a <time>').not.toBeNull();
  return found!;
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

describe('#2497: A01 times are Bangkok date + time in the screen locale', () => {
  it('row 1: as-of 2026-10-06T18:30:00.000Z shows 7 Oct 01:30 Bangkok in the header, the branch row and the alert since (th)', async () => {
    const el = mount();
    await settle();
    expect(text(headerTime(el))).toBe(AS_OF_TH);
    expect(text(rowTime(el, 'B1'))).toBe(AS_OF_TH);
    expect(text(alertTime(el))).toBe(SINCE_TH);
    expect(text(headerTime(el))).toContain('01:30');
    expect(text(headerTime(el))).not.toContain('18:30');
    expect(text(el)).not.toContain(AS_OF);
    expect(text(el)).not.toContain(SINCE);
  });

  it('row 1 (en): the same times follow the screen locale when the language is English', async () => {
    setLang('en');
    const el = mount();
    await settle();
    expect(text(headerTime(el))).toBe(AS_OF_EN);
    expect(text(rowTime(el, 'B1'))).toBe(AS_OF_EN);
    expect(text(alertTime(el))).toBe(SINCE_EN);
    expect(text(headerTime(el))).toContain('01:30');
    expect(text(headerTime(el))).not.toContain('18:30');
    expect(text(el)).not.toContain(AS_OF);
    expect(text(el)).not.toContain(SINCE);
  });

  it('row 2: the datetime attribute is still the raw server string while the visible text is Bangkok time', async () => {
    const el = mount();
    await settle();
    for (const time of [headerTime(el), rowTime(el, 'B1')]) {
      expect(time.getAttribute('datetime')).toBe(AS_OF);
      expect(text(time)).toBe(AS_OF_TH);
    }
    expect(alertTime(el).getAttribute('datetime')).toBe(SINCE);
    expect(text(alertTime(el))).toBe(SINCE_TH);
  });

  it('row 3: a null as-of shows the never text, beside a set one shown as Bangkok time', async () => {
    replyData = sampleData({ branches: [branchRow('B1', AS_OF), branchRow('B2', null)] });
    const el = mount();
    await settle();
    const r2 = el.querySelector('[data-branch="B2"]');
    expect(r2?.querySelector('time')).toBeNull();
    expect(text(r2?.querySelector('.executive-never'))).toBe(i18n.th.never);
    expect(text(rowTime(el, 'B1'))).toBe(AS_OF_TH);
  });

  it('row 4: a poll that moves a branch shows its new as-of as Bangkok time (poll race rules kept)', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    const el = mount();
    await settle();
    expect(text(rowTime(el, 'B1'))).toBe(AS_OF_TH);
    const moved = '2026-10-06T19:45:00.000Z';
    pollData = sampleData({ branches: [branchRow('B1', moved)], alerts: [] });
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(rowTime(el, 'B1').getAttribute('datetime')).toBe(moved);
    expect(text(rowTime(el, 'B1'))).toBe('7/10/2569 02:45:00');
  });

  it('edge case: an unparsable string shows as sent, not "Invalid Date", while a valid one is Bangkok time', async () => {
    replyData = sampleData({ branches: [branchRow('B1', AS_OF), branchRow('B2', 'not-a-time')] });
    const el = mount();
    await settle();
    expect(text(rowTime(el, 'B2'))).toBe('not-a-time');
    expect(text(el)).not.toContain('Invalid Date');
    expect(text(rowTime(el, 'B1'))).toBe(AS_OF_TH);
  });
});
