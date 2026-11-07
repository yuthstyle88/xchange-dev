// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// FE-T4 #5850 (docs/design/fe-triage-1101.md §3 W9, W10): the teller's waiting line shows the count difference per
// currency, and the close form says how old the shift is when it opened on an earlier Bangkok date.
// Written before the implementation by the ticket author claude/01TxaRJG.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeCashCountRow,
  makeCashCountTotalRow,
  makeDenominationRow,
  makeMyShift,
  makeMyShiftBalance,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { CountForm, CountWaitingLine } from './CountForm';
import { en, th } from './i18n';

const BRANCH = 'MAIN';
const ME = 'u-teller';
const SHIFT = 'shift-1';
const DENOMS = [makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' })];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let views: string[] = [];
let myShiftAnswer: () => Response | Promise<Response>;

function shiftOpenedAt(openedAt: string) {
  return () =>
    json(200, {
      version: 1,
      changed: true,
      data: makeMyShift({
        id: SHIFT,
        openedAt,
        balances: [makeMyShiftBalance({ code: 'THB', opening: '0', expected: '5000.00' })],
      }),
    });
}

function stub() {
  views = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const view = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        views.push(view);
        if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        if (view === 'my_shift') return myShiftAnswer();
        return json(404, { error: { code: 'notFound', message: 'x' } });
      }
      return json(200, { id: 'cc-1' });
    }),
  );
}

function mount(kind: 'OPEN' | 'CLOSE') {
  const [busy, setBusy] = createSignal(false);
  const [denoms] = createSignal(DENOMS);
  const ctx: StaffCtx = {
    me: makeStaffMeView({ userId: ME, name: 'Somchai', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions: ['ShiftOperate'] }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-10-29T01:00:00Z',
      myShift: kind === 'CLOSE' ? { id: SHIFT, openedAt: '2026-10-29T01:05:00Z' } : null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
  dispose = render(
    () => (
      <CountForm
        ctx={ctx}
        kind={kind}
        denominations={denoms}
        busy={busy}
        setBusy={setBusy}
        onSaved={vi.fn(async () => {})}
        onCancel={vi.fn()}
        onError={vi.fn()}
      />
    ),
    host,
  );
}

async function settle() {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const age = () => host.querySelector('[data-testid="shift-age"]');

describe('FE-T4 #5850 W10 CountForm shift age', () => {
  beforeEach(() => {
    // Only Date is faked, so settle()'s setTimeout still runs.
    vi.useFakeTimers({ toFake: ['Date'] });
    // 2026-11-01 10:00 ICT.
    vi.setSystemTime(new Date('2026-11-01T03:00:00Z'));
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    myShiftAnswer = shiftOpenedAt('2026-10-29T01:05:00Z');
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('A1 CLOSE, shift opened 29/10 (3 Bangkok days ago): th hint with the date and the day count', async () => {
    mount('CLOSE');
    await settle();
    expect(age()?.textContent?.trim()).toBe('กะนี้เปิดเมื่อ 29/10/2026 (3 วันก่อน)');
  });

  it('A2 CLOSE, shift opened 01/11 00:30 ICT (31/10 in UTC): today in Bangkok, no hint', async () => {
    myShiftAnswer = shiftOpenedAt('2026-10-31T17:30:00Z');
    mount('CLOSE');
    await settle();
    expect(views).toContain('my_shift');
    expect(host.querySelector('[data-testid="shift-count-form"]')).not.toBeNull();
    expect(age()).toBeNull();
  });

  it('A3 CLOSE, shift opened 31/10 23:30 ICT, now 01/11 08:00 ICT: calendar days, so 1 day ago', async () => {
    vi.setSystemTime(new Date('2026-11-01T01:00:00Z'));
    myShiftAnswer = shiftOpenedAt('2026-10-31T16:30:00Z');
    mount('CLOSE');
    await settle();
    expect(age()?.textContent?.trim()).toBe('กะนี้เปิดเมื่อ 31/10/2026 (1 วันก่อน)');
  });

  it('A4 en: "This shift opened on dd/MM/yyyy (n days ago)", and "1 day ago" for one day', async () => {
    setLang('en');
    mount('CLOSE');
    await settle();
    expect(age()?.textContent?.trim()).toBe('This shift opened on 29/10/2026 (3 days ago)');
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    myShiftAnswer = shiftOpenedAt('2026-10-31T16:30:00Z');
    mount('CLOSE');
    await settle();
    expect(age()?.textContent?.trim()).toBe('This shift opened on 31/10/2026 (1 day ago)');
  });

  it('A5 OPEN form never shows the hint', async () => {
    mount('OPEN');
    await settle();
    expect(views).not.toContain('my_shift');
    expect(host.querySelector('[data-testid="shift-count-form"]')).not.toBeNull();
    expect(age()).toBeNull();
  });

  it('A6 CLOSE, my_shift answers null or fails: no hint, the form still shows', async () => {
    myShiftAnswer = () => json(200, { version: 1, changed: true, data: null });
    mount('CLOSE');
    await settle();
    expect(host.querySelector('[data-testid="shift-count-form"]')).not.toBeNull();
    expect(age()).toBeNull();
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    myShiftAnswer = () => json(500, { error: { code: 'internal', message: 'x' } });
    mount('CLOSE');
    await settle();
    expect(host.querySelector('[data-testid="shift-count-form"]')).not.toBeNull();
    expect(age()).toBeNull();
  });
});

describe('FE-T4 #5850 W9 CountWaitingLine difference', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  const row = (totals: ReturnType<typeof makeCashCountTotalRow>[]) =>
    makeCashCountRow({ id: 'c1', kind: 'CLOSE', countedBy: ME, countedAt: '2026-10-11T04:30:00Z', totals });

  const line = () => host.querySelector('[data-testid="shift-close-waiting"]');
  const diffs = () =>
    [...host.querySelectorAll('[data-count-difference]')].map((el) => [
      el.getAttribute('data-count-difference'),
      el.textContent?.trim(),
    ]);

  it('V1 a negative THB difference: "ผลต่าง THB -1,000" after the time', () => {
    dispose = render(
      () => (
        <CountWaitingLine
          kind="CLOSE"
          row={row([makeCashCountTotalRow({ currency: 'THB', counted: '4000', expected: '5000', difference: '-1000' })])}
        />
      ),
      host,
    );
    expect(diffs()).toEqual([['THB', 'ผลต่าง THB -1,000']]);
    expect(line()?.textContent).toBe(`${th.closeWaiting} (11/10/2026 11:30) · ผลต่าง THB -1,000`);
  });

  it('V2 zero and null differences show nothing; the line reads as before', () => {
    dispose = render(
      () => (
        <CountWaitingLine
          kind="CLOSE"
          row={row([
            makeCashCountTotalRow({ currency: 'THB', counted: '5000', expected: '5000', difference: '0' }),
            makeCashCountTotalRow({ currency: 'USD', counted: '10', expected: '10', difference: '0.00' }),
            makeCashCountTotalRow({ currency: 'EUR', counted: '5', expected: null, difference: null }),
          ])}
        />
      ),
      host,
    );
    expect(diffs()).toEqual([]);
    expect(line()?.textContent).toBe(`${th.closeWaiting} (11/10/2026 11:30)`);
  });

  it('V3 a positive difference is signed "+", currencies keep the totals order, en says "Difference"', () => {
    setLang('en');
    dispose = render(
      () => (
        <CountWaitingLine
          kind="OPEN"
          row={{
            ...row([
              makeCashCountTotalRow({ currency: 'THB', counted: '6500.5', expected: '5000', difference: '1500.5' }),
              makeCashCountTotalRow({ currency: 'USD', counted: '10', expected: '10', difference: '0' }),
              makeCashCountTotalRow({ currency: 'JPY', counted: '9000', expected: '10000', difference: '-1000' }),
            ]),
            kind: 'OPEN',
          }}
        />
      ),
      host,
    );
    expect(diffs()).toEqual([
      ['THB', 'Difference THB +1,500.5'],
      ['JPY', 'Difference JPY -1,000'],
    ]);
    expect(host.querySelector('[data-testid="shift-open-waiting"]')?.textContent).toBe(
      `${en.openWaiting} (11/10/2026 11:30) · Difference THB +1,500.5 · Difference JPY -1,000`,
    );
  });
});
