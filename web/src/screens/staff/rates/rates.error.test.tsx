// S-8b #1285: the staff RATES card has an error state.
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md, row S-8b and its §8.1 test row; Blueprint §03 layout (BP:268)
//   "ทุกตารางมี loading / empty / error / retry".
// #1285 §Fix [required]:
//   - a failed read of staff_published_rates is kept and shown as errorText(code, message) (shared/i18n), with a retry;
//     while it has failed, neither the freshness line nor a rate is shown;
//   - noSourceDate (ไม่มีเรทอ้างอิง) appears only after a successful read, never after a failed one;
//   - retry reads staff_published_rates again once; a success replaces the error with the rates.
//
// What this file assumes beyond #1285 ([proposed], stated so it can be argued):
//   - the error text is in an element with role="alert" inside the card;
//   - the retry is the card's only <button> (S-8's card renders no other control: rates.test.tsx "renders no input").
//   No new i18n key name is pinned.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang, t } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeStaffMeView,
  type PublishedRatesData,
  type StaffMeView,
} from '../../../shared/testing/fixtures';
import { card } from './card';

const BRANCH = 'MAIN';
const NO_SOURCE = t('rate.noSourceDate');

function me() {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles: [],
    branchId: BRANCH,
    branchName: 'สาขาทดสอบ',
    permissions: ['CompatRead'],
  });
}

function ctx(m: StaffMeView) {
  return { me: m, floor: undefined, refresh: vi.fn(async () => {}) } as unknown as Parameters<
    typeof card.visible
  >[0];
}

/** The same published set as rates.test.tsx: strings with trailing zeros. */
const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: '11111111-2222-4333-8444-555555555555', version: 7, publishedAt: '2026-10-10T03:15:00Z' }),
  sourceDate: '2026-10-09',
  stale: false,
  lines: [
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '32.4500', sellRate: '32.9000', lineId: 'line-usd', bands: [] }),
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.0450', sellRate: '21.5926', lineId: 'line-jpy', bands: [] }),
  ],
  bandHeadings: [],
});
const RATE_STRINGS = RATES.lines.flatMap((l) => [l.buyRate, l.sellRate]).filter((r): r is string => r !== null);

/** One answer of the view, in the order the card asks. */
type Answer =
  | { kind: 'ok'; data: PublishedRatesData }
  | { kind: 'status'; status: number; body: string }
  | { kind: 'network' };

let reads = 0;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function stubAnswers(answers: Answer[]) {
  reads = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (!url.startsWith('/api/v3/views/staff_published_rates')) {
        return new Response(JSON.stringify({ error: { code: 'notFound', message: 'unexpected request' } }), {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const a = answers[Math.min(reads, answers.length - 1)]!;
      reads++;
      if (a.kind === 'network') throw new TypeError('Failed to fetch');
      if (a.kind === 'status') {
        return new Response(a.body, { status: a.status, headers: { 'Content-Type': 'application/json' } });
      }
      return new Response(JSON.stringify({ version: 3, changed: true, data: a.data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(answers: Answer[]) {
  stubAnswers(answers);
  const C = card.Component;
  const c = ctx(me());
  dispose = render(() => <C ctx={c} />, host);
  await vi.waitFor(() => expect(reads).toBeGreaterThan(0));
  await settle();
  return host;
}

/** The card after a failed read: an alert with `expected` (when given), one retry button, no freshness, no rate. */
function expectFailed(el: HTMLElement, expected?: string) {
  const text = el.textContent ?? '';
  expect(text, 'a failed read is not "no reference rate"').not.toContain(NO_SOURCE);
  for (const r of RATE_STRINGS) expect(text).not.toContain(r);
  expect(el.querySelectorAll('.xc-rates-row').length, 'no rate row').toBe(0);
  const alert = el.querySelector('[role="alert"]');
  expect(alert, 'an element with role="alert" holds the error').not.toBeNull();
  const said = (alert!.textContent ?? '').trim();
  expect(said.length, 'the alert says something').toBeGreaterThan(0);
  if (expected !== undefined) expect(said).toContain(expected);
  const buttons = el.querySelectorAll('button');
  expect(buttons.length, 'exactly one button: the retry').toBe(1);
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('S-8b a failed read is an error with a retry, never "no reference rate" (#1285, BP:268)', () => {
  it('a 500 with a v3 error body shows errorText(code, message) and a retry, no freshness line, no rate', async () => {
    const el = await mount([
      { kind: 'status', status: 500, body: JSON.stringify({ error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }) },
    ]);
    expectFailed(el, errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('outOfScope (403) shows errorText(code, message) and a retry, no freshness line, no rate', async () => {
    const el = await mount([
      {
        kind: 'status',
        status: 403,
        body: JSON.stringify({ error: { code: 'outOfScope', message: 'สาขานี้อยู่นอกสิทธิ์ของคุณ' } }),
      },
    ]);
    expectFailed(el, errorText('outOfScope', 'สาขานี้อยู่นอกสิทธิ์ของคุณ'));
  });

  it('a network error shows an error and a retry, no freshness line, no rate', async () => {
    const el = await mount([{ kind: 'network' }]);
    expectFailed(el);
  });

  it('retry reads the view once more; a success replaces the error with the rates', async () => {
    const el = await mount([
      { kind: 'status', status: 500, body: JSON.stringify({ error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }) },
      { kind: 'ok', data: RATES },
    ]);
    expectFailed(el);
    expect(reads).toBe(1);

    el.querySelector('button')!.click();
    await vi.waitFor(() => expect(reads).toBe(2));
    await settle();

    expect(el.querySelector('[role="alert"]'), 'the error is gone').toBeNull();
    const text = el.textContent ?? '';
    for (const r of RATE_STRINGS) expect(text).toContain(r);
    expect(el.querySelectorAll('.xc-rates-row').length).toBe(RATES.lines.length);
    expect(reads, 'one read per retry').toBe(2);
  });

  it('a successful read of an empty set still says "no reference rate" (the success path is unchanged)', async () => {
    const el = await mount([
      { kind: 'ok', data: makePublishedRatesData({ publication: null, sourceDate: null, stale: false, lines: [] }) },
    ]);
    expect(el.querySelector('[role="alert"]')).toBeNull();
    expect(el.textContent ?? '').toContain(NO_SOURCE);
  });
});
