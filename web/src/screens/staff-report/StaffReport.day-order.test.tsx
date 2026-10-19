import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBranchRef, makeMe } from '../../shared/testing/fixtures';
import { warmModules } from '../../shared/testing/warm-modules';

const mainMods = import.meta.glob('./main.tsx');
const pageHtml = import.meta.glob<string>('../../../pages/staff-report.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function hostId(): string {
  const raw = pageHtml['../../../pages/staff-report.html'];
  expect(raw, 'web/pages/staff-report.html exists').toBeDefined();
  const doc = new DOMParser().parseFromString(raw!, 'text/html');
  const el = doc.body.querySelector('[id]');
  expect(el, 'staff-report.html has a mount element with an id').not.toBeNull();
  return el!.id;
}

type Status = 'reconciled' | 'closedAwaitingReconciliation' | 'preliminary';
type Report = {
  day: string;
  timezone: string;
  generatedAt: string;
  company: string | null;
  site: string | null;
  branchName: string;
  status: Status;
  completed: number;
  buyVolume: string;
  sellVolume: string;
  reversedVolume: string;
  pendingTransactions: number;
  currencies: {
    currency: string;
    drawerCounted: string | null;
    drawerExpected: string | null;
    drawerDifference: string | null;
    safeCounted: string | null;
    safeExpected: string | null;
    safeDifference: string | null;
  }[];
  pendingRequests: number;
  openExceptions: number;
  notes: { kind: string; note: string }[];
  openHolds: number;
  inTransit: { kind: string; amount: string; currency: string }[];
  signers: { name: string; capacity: 'counted' | 'verified' }[];
  financeNote: string | null;
};

function report(over: Partial<Report> = {}): Report {
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

const SESSION_ME = makeMe({
  id: '00000000-0000-4000-8000-0000000000a1',
  name: 'Ploy',
  roles: [],
  permissions: ['ShiftCloseCertify'],
  branch: 'MAIN',
  branchName: 'XC-B',
  branches: [makeBranchRef({ id: 'MAIN', code: 'XC-B', name: 'XC-B' })],
});

type Answer = { status: number; body: unknown };
const ok = (data: unknown): Answer => ({ status: 200, body: { version: 1, changed: true, data } });

const REPORT_PATH = '/api/v3/views/staff_daily_report';

let reportAnswer: (url: URL) => Answer | Promise<Answer>;
let host: HTMLElement | undefined;

beforeAll(
  () =>
    warmModules(
      () => import('../../shared/i18n'),
      () => import('./main'),
    ),
  60_000,
);

beforeEach(() => {
  vi.useFakeTimers();
  reportAnswer = (url) => ok(report({ day: url.searchParams.get('day') ?? '2026-10-05' }));
  const fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    let answer: Answer;
    if (url.pathname === REPORT_PATH) {
      answer = await reportAnswer(url);
    } else if (url.pathname === '/api/v3/views/me') {
      answer = ok(SESSION_ME);
    } else if (url.pathname === '/health/ready') {
      answer = {
        status: 200,
        body: { version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-11T00:00:00Z' },
      };
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
  host?.remove();
  host = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function mountMain(): Promise<HTMLElement> {
  const id = hostId();
  const load = mainMods['./main.tsx'];
  expect(load, 'web/src/screens/staff-report/main.tsx exists').toBeDefined();
  vi.resetModules();
  const i18n = await import('../../shared/i18n');
  i18n.setLang('th');
  host = document.createElement('div');
  host.id = id;
  document.body.appendChild(host);
  await load!();
  return host;
}

const squash = (s: string | null | undefined): string => (s ?? '').replace(/\s+/g, '');
const pageText = (): string => squash(host?.textContent);

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
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('StaffReport overlapping day reads (#2243)', () => {
  it("picks day A then day B: when A's answer arrives after B's, report and date box show day B", async () => {
    const dayA = deferred<Answer>();
    const dayB = deferred<Answer>();
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      if (d === '2026-10-14') return dayA.promise;
      if (d === '2026-10-15') return dayB.promise;
      return ok(report({ day: '2026-10-05', completed: 5 }));
    };

    await mountMain();
    await flush();
    expect(pageText()).toContain(squash('XC Silom · 2026-10-05'));

    await changeDay('2026-10-14');
    await changeDay('2026-10-15');

    dayB.resolve(ok(report({ day: '2026-10-15', completed: 15 })));
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));

    dayA.resolve(ok(report({ day: '2026-10-14', completed: 14 })));
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));
    expect(pageText()).not.toContain(squash('XC Silom · 2026-10-14'));
    expect(pageText()).not.toContain(squash('รายการสำเร็จ 14'));
  });

  it("picks day A then day B: when A's read fails after B's answer, day B's report stays with no refused state", async () => {
    const dayA = deferred<Answer>();
    const dayB = deferred<Answer>();
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      if (d === '2026-10-14') return dayA.promise;
      if (d === '2026-10-15') return dayB.promise;
      return ok(report({ day: '2026-10-05', completed: 5 }));
    };

    await mountMain();
    await flush();

    await changeDay('2026-10-14');
    await changeDay('2026-10-15');

    dayB.resolve(ok(report({ day: '2026-10-15', completed: 15 })));
    await flush();
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));

    dayA.resolve({
      status: 400,
      body: { error: { code: 'roleNotAllowed', message: 'role not allowed' } },
    });
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));
    expect(pageText()).not.toContain(squash('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน'));
    expect(host!.querySelector('.staff-report-refusal')).toBeNull();
  });

  it('first load (no day chosen) followed by a fast day pick: the picked day wins when initial load finishes later', async () => {
    const initialLoad = deferred<Answer>();
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      if (!d) return initialLoad.promise;
      return ok(report({ day: d, completed: 15 }));
    };

    await mountMain();
    await flush();

    await changeDay('2026-10-15');
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));

    initialLoad.resolve(ok(report({ day: '2026-10-05', completed: 5 })));
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-15'));
    expect(pageText()).toContain(squash('รายการสำเร็จ 15'));
    expect(pageText()).not.toContain(squash('XC Silom · 2026-10-05'));
    expect(pageText()).not.toContain(squash('รายการสำเร็จ 5'));
  });

  it('shows roleNotAllowed refusal when the latest pick is refused, even if an earlier pick resolves afterwards', async () => {
    const dayA = deferred<Answer>();
    reportAnswer = (url) => {
      const d = url.searchParams.get('day');
      if (d === '2026-10-14') return dayA.promise;
      if (d === '2026-10-15') {
        return {
          status: 400,
          body: { error: { code: 'roleNotAllowed', message: 'role not allowed' } },
        };
      }
      return ok(report({ day: '2026-10-05', completed: 5 }));
    };

    await mountMain();
    await flush();

    await changeDay('2026-10-14');
    await changeDay('2026-10-15');
    await flush();
    expect(pageText()).toContain(squash('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน'));

    dayA.resolve(ok(report({ day: '2026-10-14', completed: 14 })));
    await flush();
    expect(dayInput().value).toBe('2026-10-15');
    expect(pageText()).toContain(squash('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน'));
    expect(pageText()).not.toContain(squash('XC Silom · 2026-10-14'));
  });
});
