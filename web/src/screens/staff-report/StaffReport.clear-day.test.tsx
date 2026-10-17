import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import type { StaffDailyReportData } from '../../shared/api/generated/staff_daily_report';
import { StaffReport } from './StaffReport';

type Answer = { status: number; body: unknown };
const ok = (data: unknown): Answer => ({ status: 200, body: { version: 1, changed: true, data } });

function report(over: Partial<StaffDailyReportData> = {}): StaffDailyReportData {
  return {
    day: '2026-10-05',
    timezone: 'Asia/Bangkok',
    generatedAt: '2026-10-05T12:34:56Z',
    company: 'XC Exchange Co., Ltd.',
    site: 'XC Silom',
    branchName: 'XC-B',
    status: 'closedAwaitingReconciliation',
    completed: 7,
    buyVolume: '1000.00',
    sellVolume: '500.00',
    reversedVolume: '0.00',
    pendingTransactions: 0,
    currencies: [
      {
        currency: 'THB',
        drawerCounted: '1000.00',
        drawerExpected: '1000.00',
        drawerDifference: '0.00',
        safeCounted: '5000.00',
        safeExpected: '5000.00',
        safeDifference: '0.00',
      },
    ],
    pendingRequests: 0,
    openExceptions: 0,
    notes: [],
    openHolds: 0,
    inTransit: [],
    signers: [{ name: 'Ploy', capacity: 'counted' }],
    financeNote: 'ok',
    ...over,
  };
}

const REPORT_PATH = '/api/v3/views/staff_daily_report';

let reportAnswer: (url: URL) => Answer | Promise<Answer>;
let host: HTMLElement | undefined;
let dispose: (() => void) | undefined;
let requestedDays: (string | null)[];

beforeEach(() => {
  vi.useFakeTimers();
  setLang('th');
  requestedDays = [];
  reportAnswer = (url) => {
    const d = url.searchParams.get('day');
    requestedDays.push(d);
    return ok(report({ day: d ?? '2026-10-05' }));
  };

  const fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    let answer: Answer;
    if (url.pathname === REPORT_PATH) {
      answer = await reportAnswer(url);
    } else {
      answer = { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    }
    return new Response(JSON.stringify(answer.body), {
      status: answer.status,
      headers: { 'content-type': 'application/json' },
    });
  });
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function mountScreen(): Promise<HTMLElement> {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <StaffReport />, host);
  await flush();
  return host;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

function dayInput(): HTMLInputElement {
  const input = host!.querySelector('input[type="date"]') as HTMLInputElement | null;
  expect(input, 'the day input').not.toBeNull();
  return input!;
}

async function changeDay(value: string): Promise<void> {
  const input = dayInput();
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await flush();
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('StaffReport clearing date box (#2889)', () => {
  it('1: pick 2026-10-01, then clear date box: reads default day, updates box and sheet', async () => {
    await mountScreen();
    expect(dayInput().value).toBe('2026-10-05');

    // Pick 2026-10-01
    await changeDay('2026-10-01');
    expect(dayInput().value).toBe('2026-10-01');
    expect(host!.querySelector('.staff-report-sheet')).not.toBeNull();

    requestedDays = [];
    // Clear the date box
    await changeDay('');

    // A read must be sent with no day filter (null in searchParams)
    expect(requestedDays).toEqual([null]);
    // The box is set to the day the server answers (default day 2026-10-05)
    expect(dayInput().value).toBe('2026-10-05');
  });

  it('2: pick 2026-10-15 whose read fails (500), then clear date box: refusal goes away and default report shown', async () => {
    await mountScreen();

    // Pick 2026-10-15 and fail
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      requestedDays.push(d);
      if (d === '2026-10-15') {
        return {
          status: 500,
          body: { error: { code: 'internal', message: 'internal server error' } },
        };
      }
      return ok(report({ day: d ?? '2026-10-05' }));
    };

    await changeDay('2026-10-15');
    expect(host!.querySelector('.staff-report-refusal')).not.toBeNull();
    expect(host!.querySelector('.staff-report-sheet')).toBeNull();

    // Clear the date box
    await changeDay('');

    expect(host!.querySelector('.staff-report-refusal')).toBeNull();
    expect(host!.querySelector('.staff-report-sheet')).not.toBeNull();
    expect(dayInput().value).toBe('2026-10-05');
  });

  it('3: pick 2026-10-14, clear while in flight, then 2026-10-14 arrives: default-day answer wins', async () => {
    await mountScreen();

    const dayPending = deferred<Answer>();
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      requestedDays.push(d);
      if (d === '2026-10-14') {
        return dayPending.promise;
      }
      return ok(report({ day: d ?? '2026-10-05', company: 'Default Day Co.' }));
    };

    // Pick 2026-10-14 (in flight)
    const input = dayInput();
    input.value = '2026-10-14';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    // Clear the box while read is still in flight
    input.value = '';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    // Now 2026-10-14 answer arrives
    dayPending.resolve(ok(report({ day: '2026-10-14', company: 'Late 10-14 Co.' })));
    await flush();

    // Default day answer wins; late 2026-10-14 answer changes neither box nor sheet
    expect(dayInput().value).toBe('2026-10-05');
    expect(host!.textContent).toContain('Default Day Co.');
    expect(host!.textContent).not.toContain('Late 10-14 Co.');
  });
});
