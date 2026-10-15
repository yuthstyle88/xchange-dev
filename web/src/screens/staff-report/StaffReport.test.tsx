// V3-staff-report #1350: the /staff/report screen, mounted through its real entry `staff-report/main.tsx` inside the
// shared AppShell (design docs/design/xchange-v3-wave3-staff-report.md §B.1 rows 1–22, §B.6 "Screen", D-B1, D-B4,
// D-B6, D-B8; owner answers Q-SR1…Q-SR4 of 2026-10-12). v2's look is xchange-v2-source@4d0bdbb
// `src/booking/app/staff/report/page.tsx`; every Thai string below is copied from it byte for byte, except
// `มูลค่าแลก · ร้านซื้อ` / `มูลค่าแลก · ร้านขาย` (B.1 row 11, known difference) and `ผู้นับ` / `ผู้รับรอง` (D-B5).
// Written before the implementation by the up-front test writer (claude/14cc24bd, on the xchange FE lane controller
// claude/5edf7bb5's order). Do not edit, delete or weaken these tests; add your own in another file.
//
// The surface these tests assume, and nothing more:
// - `web/pages/staff-report.html` mounts `/src/screens/staff-report/main.tsx` into the element with an `id` in its body;
// - main.tsx renders the screen inside `<AppShell>` (D-B8, LK-1) and reads through the global `fetch` (an `ApiClient`
//   built without a fetch argument, or the shared session's client), so this file stubs `fetch` with a route table;
// - the screen reads `GET /api/v3/views/staff_daily_report` (`ApiClient.view('staff_daily_report', { filter: { day } })`)
//   on mount with no `day`, and again with `day=<YYYY-MM-DD>` on a change of its `<input type="date">`; nothing else;
// - `i18n.ts` exports `th` and `en` tables with the same keys (D-B6, W10).
// The not-yet-written modules are reached through import.meta.glob, so their absence fails an assertion, not the
// type check.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '../../shared/api/generated/common';
import { formatMoney } from '../../shared/ui/MoneyText';
import { warmModules } from '../../shared/testing/warm-modules';

// ── the modules under test ──────────────────────────────────────────────────────────────────

const mainMods = import.meta.glob('./main.tsx');
const textMods = import.meta.glob<{ th: Record<string, string>; en: Record<string, string> }>('./i18n.ts', {
  eager: true,
});
const sourceMods = import.meta.glob<string>(['./StaffReport.tsx', './main.tsx'], {
  query: '?raw',
  import: 'default',
  eager: true,
});
const pageHtml = import.meta.glob<string>('../../../pages/staff-report.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function texts(): { th: Record<string, string>; en: Record<string, string> } {
  const m = textMods['./i18n.ts'];
  expect(m, 'web/src/screens/staff-report/i18n.ts exists').toBeDefined();
  expect(m!.th, 'i18n.ts exports th').toBeTypeOf('object');
  expect(m!.en, 'i18n.ts exports en').toBeTypeOf('object');
  return { th: m!.th, en: m!.en };
}

/** The id of the element web/pages/staff-report.html gives main.tsx to mount into. */
function hostId(): string {
  const raw = pageHtml['../../../pages/staff-report.html'];
  expect(raw, 'web/pages/staff-report.html exists').toBeTypeOf('string');
  const doc = new DOMParser().parseFromString(raw!, 'text/html');
  const el = doc.body.querySelector('[id]');
  expect(el, 'staff-report.html has a mount element with an id').not.toBeNull();
  return el!.id;
}

// ── the view's answer (the shape the backend tests pin, B.3 StaffDailyReportData) ───────────

type Status = 'reconciled' | 'closedAwaitingReconciliation' | 'preliminary';
type CurrencyRow = {
  currency: string;
  drawerCounted: string | null;
  drawerExpected: string | null;
  drawerDifference: string | null;
  safeCounted: string | null;
  safeExpected: string | null;
  safeDifference: string | null;
};
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
  currencies: CurrencyRow[];
  pendingRequests: number;
  openExceptions: number;
  notes: { kind: string; note: string }[];
  openHolds: number;
  inTransit: { kind: string; amount: string; currency: string }[];
  signers: { name: string; capacity: 'counted' | 'verified' }[];
  financeNote: string | null;
};

const CODES = ['THB', 'USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'] as const;

function row(currency: string, base: number): CurrencyRow {
  // Values no rounding or arithmetic would leave alone: 4 decimals with trailing zeros, and differences that are
  // NOT counted − expected (the stored difference is printed, never recomputed: B.1 row 13, D-B4).
  return {
    currency,
    drawerCounted: `${base}1234.5600`,
    drawerExpected: `${base}1234.5650`,
    drawerDifference: '7.7700',
    safeCounted: `${base}0007.1000`,
    safeExpected: `${base}0007.0990`,
    safeDifference: '-3.3000',
  };
}

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
    buyVolume: '1234567.8900',
    sellVolume: '0.10',
    reversedVolume: '999.999',
    pendingTransactions: 3,
    currencies: CODES.map((c, i) => row(c, i + 1)),
    pendingRequests: 4,
    openExceptions: 2,
    notes: [
      { kind: 'CLOSE', note: 'drawer short by one note' },
      { kind: 'BRANCH', note: 'sent with guard' },
    ],
    openHolds: 5,
    inTransit: [
      { kind: 'BRANCH', amount: '50000.00', currency: 'USD' },
      { kind: 'DRAWER_TO_SAFE', amount: '1200.5', currency: 'THB' },
    ],
    signers: [
      { name: 'Ploy', capacity: 'counted' },
      { name: 'Mint', capacity: 'verified' },
    ],
    financeNote: 'checked against the bank slip',
    ...over,
  };
}

// ── the fake server behind the global fetch ─────────────────────────────────────────────────

const SESSION_ME: Me = {
  id: '00000000-0000-4000-8000-0000000000a1',
  name: 'Ploy',
  roles: [],
  permissions: ['ShiftCloseCertify'],
  branch: 'MAIN',
  branchName: 'XC-B',
  branches: [{ id: 'MAIN', code: 'XC-B', name: 'XC-B' }],
};

type Answer = { status: number; body: unknown };
const ok = (data: unknown): Answer => ({ status: 200, body: { version: 1, changed: true, data } });

const REPORT_PATH = '/api/v3/views/staff_daily_report';

let reportAnswer: (url: URL) => Answer | Promise<Answer>;
let reportCalls: URL[];
let fetchSpy: ReturnType<typeof vi.fn>;
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
  reportCalls = [];
  reportAnswer = (url) => ok(report({ day: url.searchParams.get('day') ?? '2026-10-05' }));
  fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    let answer: Answer;
    if (url.pathname === REPORT_PATH) {
      reportCalls.push(url);
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

/** Mount the page as its HTML entry does: a fresh module graph, the language chosen before the page starts. */
async function mountMain(lang: 'th' | 'en' = 'th'): Promise<HTMLElement> {
  const id = hostId();
  const load = mainMods['./main.tsx'];
  expect(load, 'web/src/screens/staff-report/main.tsx exists').toBeDefined();
  vi.resetModules();
  // the fresh module graph's i18n is the one main.tsx will import: choose the language there first
  const i18n = await import('../../shared/i18n');
  i18n.setLang(lang);
  host = document.createElement('div');
  host.id = id;
  document.body.appendChild(host);
  await load!();
  return host;
}

/** Text with every whitespace run removed, so `รายการสำเร็จ <b>7</b>` and `รายการสำเร็จ 7` compare equal. */
const squash = (s: string | null | undefined): string => (s ?? '').replace(/\s+/g, '');

function mainArea(): Element {
  const area = host!.querySelector('.xc-shell .xc-shell-main, .xc-shell .xc-content');
  expect(area, "the shell's main area").not.toBeNull();
  return area!;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

const pageText = (): string => squash(host?.textContent);

async function loaded(): Promise<void> {
  await flush();
  expect(pageText()).toContain(squash('รายการสำเร็จ'));
}

/** The table row whose first cell is the currency code. */
function currencyRow(code: string): HTMLTableRowElement {
  const rows = Array.from(host!.querySelectorAll('tbody tr')) as HTMLTableRowElement[];
  const found = rows.find((r) => squash(r.cells[0]?.textContent) === code);
  expect(found, `a table row for ${code}`).toBeDefined();
  return found!;
}

async function changeDay(value: string): Promise<void> {
  const input = host!.querySelector('input[type="date"]') as HTMLInputElement | null;
  expect(input, 'the day input').not.toBeNull();
  input!.value = value;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
  input!.dispatchEvent(new Event('change', { bubbles: true }));
}

const thai = /[฀-๿]/;

// ── B.1: v2's strings, in th ────────────────────────────────────────────────────────────────

describe('V3-staff-report screen: v2 strings in th (B.1, D-B6)', () => {
  it('shows กำลังโหลดรายงาน… while the report is loading (row 3)', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    reportAnswer = async () => {
      await gate;
      return ok(report());
    };
    await mountMain();
    await flush();
    expect(pageText()).toContain(squash('กำลังโหลดรายงาน…'));
    release();
    await loaded();
    expect(pageText()).not.toContain(squash('กำลังโหลดรายงาน…'));
  });

  it('prints every v2 line of the sheet with the answer’s figures, as given (rows 9–22)', async () => {
    await mountMain();
    await loaded();
    const text = pageText();
    const r = report();
    const expected = [
      // the tools (row 4, 21, 22)
      'วันที่',
      'พิมพ์ / บันทึก PDF',
      'กลับ',
      // the heading (row 9)
      'DAILY CLOSE · รายงานปิดวัน',
      'XC Exchange Co., Ltd.',
      'XC Silom · 2026-10-05',
      'ปิดกะแล้ว · รอกระทบยอด · สาขา XC-B',
      // the metrics (rows 10–12)
      'รายการสำเร็จ 7',
      `มูลค่าแลก · ร้านซื้อ ${formatMoney(r.buyVolume)} THB`,
      `มูลค่าแลก · ร้านขาย ${formatMoney(r.sellVolume)} THB`,
      'รายการธุรกรรมค้าง 3',
      // the table head (row 13)
      'สกุล',
      'ลิ้นชักนับจริงปิดวัน',
      'เซฟนับจริงปิดวัน',
      'ผลต่างลิ้นชักรวม',
      'ผลต่างเซฟรวม',
      // hand-over and exceptions (rows 14–17)
      'การส่งมอบและเหตุที่ต้องตรวจ',
      'งานรับ–ส่งรอยืนยัน 4 รายการ · เหตุผลต่างที่ยังเปิด 2 รายการ',
      'CLOSE: drawer short by one note',
      'BRANCH: sent with guard',
      'ลูกค้ารอ Monitor ตรวจ 5 รายการ',
      'เงินระหว่างนำส่ง: BRANCH 50,000.00 USD / DRAWER_TO_SAFE 1,200.5 THB · ดูหลักฐานและหมายเหตุทั้งหมดในหน้างานประจำกะ',
      // signers and finance (rows 18–19)
      'ผู้รับรองปิดวัน',
      'Ploy (ผู้นับ) / Mint (ผู้รับรอง)',
      'Finance: checked against the bank slip',
      // the fixed note and the footer (row 20)
      'ยอดแลกและรับเข้า − จ่ายออกไม่ใช่กำไร · Rate Margin หลังต้นทุนเงินจริง: ยังไม่มีต้นทุนรายล็อตครบ · กำไรสุทธิ: ยังไม่คำนวณ',
      'จัดทำจากข้อมูลสาขาที่เลือก · สร้างรายงาน',
      'ไม่ใช่แบบรายงานส่งหน่วยงานกำกับ',
    ];
    for (const s of expected) expect(text, s).toContain(squash(s));
    // the reversed volume is shown apart, as given (B.0 "reversals shown separately"; its label is not pinned)
    expect(text).toContain(squash(formatMoney(r.reversedVolume)));
    // the back link (row 22)
    const back = Array.from(host!.querySelectorAll('a')).find((a) => squash(a.textContent) === 'กลับ');
    expect(back?.getAttribute('href')).toBe('/staff');
  });

  it('prints the fallbacks: no company, no site, no finance note (rows 9, 19)', async () => {
    reportAnswer = () => ok(report({ company: null, site: null, financeNote: null }));
    await mountMain();
    await loaded();
    const text = pageText();
    expect(text).toContain(squash('ยังไม่ยืนยันนิติบุคคล'));
    expect(text, 'the branch name stands in for the site').toContain(squash('XC-B · 2026-10-05'));
    expect(text).toContain(squash('Finance: รอกระทบยอดจากหลักฐานอิสระ'));
  });

  it("formats generatedAt th-TH in the answer's timezone, not the browser's (row 20)", async () => {
    const at = '2026-10-05T16:30:00Z'; // 23:30 in Bangkok, 01:30 the next day in Tokyo
    reportAnswer = () => ok(report({ generatedAt: at, timezone: 'Asia/Tokyo' }));
    await mountMain();
    await loaded();
    const tokyo = new Date(at).toLocaleString('th-TH', { timeZone: 'Asia/Tokyo' });
    const bangkok = new Date(at).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    expect(pageText()).toContain(squash(`สร้างรายงาน ${tokyo}`));
    expect(pageText()).not.toContain(squash(bangkok));
  });

  it('the screen sources hold no Thai letters: every text comes from i18n.ts (D-B6)', () => {
    const entries = Object.entries(sourceMods);
    expect(entries.map(([f]) => f).sort(), 'StaffReport.tsx and main.tsx exist').toEqual([
      './StaffReport.tsx',
      './main.tsx',
    ]);
    for (const [file, src] of entries) expect(src, `${file} has no Thai letters`).not.toMatch(thai);
  });
});

// ── B.1 row 8: the three status words ───────────────────────────────────────────────────────

describe('V3-staff-report screen: status words (row 8)', () => {
  it.each([
    ['reconciled', 'กระทบยอดแล้ว'],
    ['closedAwaitingReconciliation', 'ปิดกะแล้ว · รอกระทบยอด'],
    ['preliminary', 'ยอดเบื้องต้น · ยังไม่ปิดกะ'],
  ] as const)('%s → %s · สาขา <branch>', async (status, word) => {
    reportAnswer = () => ok(report({ status }));
    await mountMain();
    await loaded();
    expect(pageText()).toContain(squash(`${word} · สาขา XC-B`));
    const others = ['กระทบยอดแล้ว', 'ปิดกะแล้ว · รอกระทบยอด', 'ยอดเบื้องต้น · ยังไม่ปิดกะ'].filter((w) => w !== word);
    for (const o of others) expect(pageText()).not.toContain(squash(`${o} · สาขา`));
  });
});

// ── B.1 row 13: the currency table ──────────────────────────────────────────────────────────

describe('V3-staff-report screen: the currency table (row 13, D-B4)', () => {
  it("lists the answer's rows in v2's order, values printed as given (formatMoney only, no sum, no rounding)", async () => {
    await mountMain();
    await loaded();
    const firsts = Array.from(host!.querySelectorAll('tbody tr')).map((r) =>
      squash((r as HTMLTableRowElement).cells[0]?.textContent),
    );
    expect(firsts).toEqual([...CODES]);
    const usd = squash(currencyRow('USD').textContent);
    const r = report().currencies[1]!;
    for (const v of [r.drawerCounted, r.drawerDifference, r.safeCounted, r.safeDifference]) {
      expect(usd).toContain(squash(formatMoney(v!)));
    }
    // counted − expected would give -0.005 / 0.001: the screen must not recompute the difference
    expect(usd).not.toContain('-0.005');
    expect(usd).not.toContain('0.0010');
    expect(usd).not.toContain(squash('รอนับ'));
  });

  it('shows รอนับ for a null drawer value and for a null safe value', async () => {
    const rows = CODES.map((c, i) => row(c, i + 1));
    rows[1] = { ...rows[1]!, drawerCounted: null, drawerExpected: null, drawerDifference: null }; // USD drawer
    rows[2] = { ...rows[2]!, safeCounted: null, safeExpected: null, safeDifference: null }; // EUR safe
    rows[3] = { ...rows[3]!, drawerDifference: null }; // GBP: only the drawer difference
    rows[4] = { ...rows[4]!, safeCounted: null }; // JPY: only the safe count
    reportAnswer = () => ok(report({ currencies: rows }));
    await mountMain();
    await loaded();
    for (const code of ['USD', 'EUR', 'GBP', 'JPY']) {
      expect(squash(currencyRow(code).textContent), `${code} row`).toContain(squash('รอนับ'));
    }
    expect(squash(currencyRow('THB').textContent), 'a full row has no รอนับ').not.toContain(squash('รอนับ'));
    // the non-null values of a half-null row are still printed
    expect(squash(currencyRow('USD').textContent)).toContain(squash(formatMoney(rows[1]!.safeCounted!)));
    expect(squash(currencyRow('EUR').textContent)).toContain(squash(formatMoney(rows[2]!.drawerCounted!)));
  });
});

// ── B.1 rows 17–18: in transit and signers ──────────────────────────────────────────────────

describe('V3-staff-report screen: in transit and signers (rows 17, 18)', () => {
  it('prints ไม่มี for an empty inTransit', async () => {
    reportAnswer = () => ok(report({ inTransit: [] }));
    await mountMain();
    await loaded();
    expect(pageText()).toContain(squash('เงินระหว่างนำส่ง: ไม่มี · ดูหลักฐานและหมายเหตุทั้งหมดในหน้างานประจำกะ'));
  });

  it('prints the signer fallback when there is no signer', async () => {
    reportAnswer = () => ok(report({ signers: [] }));
    await mountMain();
    await loaded();
    expect(pageText()).toContain(squash('รอผู้จัดการและผู้ตรวจรับรอง'));
    expect(pageText()).not.toContain(squash('(ผู้นับ)'));
  });

  it('prints each signer with the capacity, ผู้นับ or ผู้รับรอง, in the answer’s order', async () => {
    reportAnswer = () =>
      ok(
        report({
          signers: [
            { name: 'Mint', capacity: 'verified' },
            { name: 'Ploy', capacity: 'counted' },
            { name: 'Nok', capacity: 'counted' },
          ],
        }),
      );
    await mountMain();
    await loaded();
    expect(pageText()).toContain(squash('Mint (ผู้รับรอง) / Ploy (ผู้นับ) / Nok (ผู้นับ)'));
    expect(pageText()).not.toContain(squash('รอผู้จัดการและผู้ตรวจรับรอง'));
  });
});

// ── B.1 rows 1–2: refusals ──────────────────────────────────────────────────────────────────

describe('V3-staff-report screen: refusals (rows 1, 2)', () => {
  function link(): HTMLAnchorElement | undefined {
    return Array.from(host!.querySelectorAll('a')).find((a) => squash(a.textContent) === squash('กลับเข้าสู่ระบบ'));
  }

  it("roleNotAllowed (400) → v2's sentence and กลับเข้าสู่ระบบ → /staff, no figure", async () => {
    reportAnswer = () => ({
      status: 400,
      body: { error: { code: 'roleNotAllowed', message: 'role not allowed' } },
    });
    await mountMain();
    await flush();
    expect(pageText()).toContain(squash('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน'));
    expect(link(), 'the กลับเข้าสู่ระบบ link').toBeDefined();
    expect(link()!.getAttribute('href')).toBe('/staff');
    expect(host!.querySelector('table'), 'no figure table').toBeNull();
    expect(pageText()).not.toContain(squash('รายการสำเร็จ'));
    expect(pageText()).not.toContain(squash('กำลังโหลดรายงาน…'));
  });

  it('any other refusal → its sentence, with the same link', async () => {
    reportAnswer = () => ({ status: 401, body: { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } } });
    await mountMain();
    await flush();
    expect(pageText()).toContain(squash('กรุณาเข้าสู่ระบบ'));
    expect(pageText()).not.toContain(squash('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน'));
    expect(link()?.getAttribute('href')).toBe('/staff');
    expect(host!.querySelector('table')).toBeNull();
  });
});

// ── D-B1, row 4: reads ──────────────────────────────────────────────────────────────────────

describe('V3-staff-report screen: reads (D-B1, row 4)', () => {
  it('reads once on mount with no day (the server decides today), and shows the answered day in the input', async () => {
    reportAnswer = () => ok(report({ day: '2026-10-03' }));
    await mountMain();
    await loaded();
    expect(reportCalls).toHaveLength(1);
    expect(reportCalls[0]!.searchParams.has('day'), 'no browser-clock day is sent').toBe(false);
    const input = host!.querySelector('input[type="date"]') as HTMLInputElement | null;
    expect(input?.value).toBe('2026-10-03');
  });

  it('a day change re-reads with day=<YYYY-MM-DD> and shows that day', async () => {
    await mountMain();
    await loaded();
    expect(reportCalls).toHaveLength(1);
    await changeDay('2026-10-01');
    await flush();
    expect(reportCalls.length).toBeGreaterThan(1);
    for (const c of reportCalls.slice(1)) expect(c.searchParams.get('day')).toBe('2026-10-01');
    expect(pageText()).toContain(squash('XC Silom · 2026-10-01'));
  });

  it('makes no request after mount without a day change (no polling)', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    await mountMain();
    await loaded();
    expect(reportCalls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(reportCalls, 'no report read in 10 minutes without a day change').toHaveLength(1);
  });
});

// ── row 21, D-B8: print and shell ───────────────────────────────────────────────────────────

describe('V3-staff-report screen: print and shell (row 21, D-B8)', () => {
  it('พิมพ์ / บันทึก PDF calls window.print', async () => {
    const print = vi.spyOn(window, 'print').mockImplementation(() => {});
    await mountMain();
    await loaded();
    const button = Array.from(host!.querySelectorAll('button')).find(
      (b) => squash(b.textContent) === squash('พิมพ์ / บันทึก PDF'),
    );
    expect(button, 'the print button').toBeDefined();
    expect(print).not.toHaveBeenCalled();
    button!.click();
    expect(print).toHaveBeenCalledTimes(1);
  });

  it('is rendered inside the shared AppShell, one shell, the report in its main area', async () => {
    await mountMain();
    await loaded();
    expect(host!.querySelectorAll('.xc-shell')).toHaveLength(1);
    await flush();
    expect(host!.querySelector('.xc-shell .account-bar'), "the shell's account bar").not.toBeNull();
    const area = squash(mainArea().textContent);
    expect(area).toContain(squash('รายการสำเร็จ 7'));
    expect(area).toContain(squash('พิมพ์ / บันทึก PDF'));
  });
});

// ── W10: en ─────────────────────────────────────────────────────────────────────────────────

describe('V3-staff-report screen: en (D-B6, W10)', () => {
  it('i18n.ts has th and en tables with the same keys, none empty, and holds v2’s Thai lines', () => {
    const { th, en } = texts();
    expect(Object.keys(th).length).toBeGreaterThan(0);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of [...Object.entries(th), ...Object.entries(en)]) expect(v.trim(), k).not.toBe('');
    const values = Object.values(th).map(squash).join('\n');
    for (const s of [
      'บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน',
      'กลับเข้าสู่ระบบ',
      'กำลังโหลดรายงาน…',
      'พิมพ์ / บันทึก PDF',
      'กระทบยอดแล้ว',
      'ปิดกะแล้ว · รอกระทบยอด',
      'ยอดเบื้องต้น · ยังไม่ปิดกะ',
      'รอนับ',
      'ไม่มี',
      'รอผู้จัดการและผู้ตรวจรับรอง',
      'รอกระทบยอดจากหลักฐานอิสระ',
      'ยังไม่ยืนยันนิติบุคคล',
      'ผู้นับ',
      'ผู้รับรอง',
    ]) {
      expect(values, `th holds ${s}`).toContain(squash(s));
    }
  });

  it('in en: no Thai letter in the report, and every th text shown in th is shown in its en text', async () => {
    const { th, en } = texts();
    const rows = CODES.map((c, i) => row(c, i + 1));
    rows[1] = { ...rows[1]!, drawerCounted: null };
    reportAnswer = () => ok(report({ company: null, financeNote: null, inTransit: [], currencies: rows }));

    await mountMain('th');
    await loaded();
    const thText = squash(mainArea().textContent);
    host!.remove();

    await mountMain('en');
    await flush();
    expect(host!.querySelector('tbody tr')).not.toBeNull();
    const enText = squash(mainArea().textContent);
    expect(enText, 'the en report has no Thai letter').not.toMatch(thai);

    // every th entry shown in the th page (each literal part of it) is shown by its en entry in the en page;
    // an entry whose text is also inside another th entry is ambiguous on the page, and is skipped
    const parts = (v: string) =>
      v
        .split(/\{\w+\}/)
        .map(squash)
        .filter((p) => p.length >= 2);
    const ambiguous = (key: string, v: string) =>
      Object.entries(th).some(([k, o]) => k !== key && squash(o) !== squash(v) && squash(o).includes(squash(v)));
    let shown = 0;
    for (const [key, thValue] of Object.entries(th)) {
      const thParts = parts(thValue);
      if (thParts.length === 0 || ambiguous(key, thValue) || !thParts.every((p) => thText.includes(p))) continue;
      shown += 1;
      for (const p of parts(en[key]!)) expect(enText, `en.${key}`).toContain(p);
    }
    expect(shown, 'th entries found on the th page').toBeGreaterThanOrEqual(10);
  });

  it('in en: the three status words, the refusal and the loading line are the en texts', async () => {
    const { th, en } = texts();
    const keyOf = (thai: string) => {
      const k = Object.keys(th).find((key) => squash(th[key]) === squash(thai));
      expect(k, `a th key whose text is ${thai}`).toBeDefined();
      return k!;
    };
    for (const [status, word] of [
      ['reconciled', 'กระทบยอดแล้ว'],
      ['closedAwaitingReconciliation', 'ปิดกะแล้ว · รอกระทบยอด'],
      ['preliminary', 'ยอดเบื้องต้น · ยังไม่ปิดกะ'],
    ] as const) {
      reportAnswer = () => ok(report({ status }));
      await mountMain('en');
      await flush();
      expect(host!.querySelector('tbody tr')).not.toBeNull();
      expect(squash(mainArea().textContent)).toContain(squash(en[keyOf(word)]));
      host!.remove();
    }

    reportAnswer = () => ({ status: 400, body: { error: { code: 'roleNotAllowed', message: 'x' } } });
    await mountMain('en');
    await flush();
    expect(squash(host!.textContent)).toContain(squash(en[keyOf('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน')]));
    expect(squash(host!.textContent)).toContain(squash(en[keyOf('กลับเข้าสู่ระบบ')]));
    host!.remove();

    reportAnswer = () => new Promise<Answer>(() => {});
    await mountMain('en');
    await flush();
    expect(squash(host!.textContent)).toContain(squash(en[keyOf('กำลังโหลดรายงาน…')]));
  });
});
