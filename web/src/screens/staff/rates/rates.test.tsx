// S-8 #574: the staff RATES card shows the branch's published rate set, read only.
// Written before the implementation by the up-front test writer (claude/5d45b017, on the xchange FE lane controller
// claude/5edf7bb5's order; the plan is ruled on #574, issuecomment-6101726258). Do not edit, delete or weaken.
//
// Design: docs/design/xchange-v3-wave3-staff.md
//   S10  "No rate is written from /staff. The RATES card shows the branch's published rate set read only; its view is
//        the rate design's staff_published_rates (rates.md:420)."  (K5: v2's RATES employee saved rates here.)
//   B11  shown exactly to those the view answers: today can(CompatRead).
//   S2   no role literal anywhere; access is a Permission.   S5  nothing through /api/staff/rpc.
//   S8   every text through t(), TH and EN.   §8.1  no Number-constructor call in web/src/screens/staff/** (money stays a string).
// rates.md §4.10: the staff RATES card embeds the shared RateFreshness (stale, sourceDate, publishedAt).
//
// The contract this test assumes ([required], from S-3 #575's card contract, SD:§4):
//   web/src/screens/staff/rates/card.ts   export const card: {
//       id: string; order: number;
//       visible(ctx): boolean;                      // reads only can(ctx.me, …) and ctx.floor
//       Component: (p: { ctx }) => JSX.Element;
//   }
//   ctx = { me: StaffMe; floor: StaffFloor | undefined; refresh: () => Promise<void> }
//   StaffMe = { userId, name, roles: {role, branchId?}[], branchId, branchName, permissions: string[] }
//   permissions are spelled as serde writes the Permission variant ("CompatRead"), as S-1 #573 pins.
//   web/src/screens/staff/rates/i18n.ts   export const th = {…}; export const en = {…}  (same keys; staff.rates.*)
//
// The card reads GET /api/v3/views/staff_published_rates?branch=<me.branchId> through ApiClient (global fetch),
// whose answer is { version, changed, data: PublishedRatesData } (web/src/shared/api/generated/published_rates.ts).
// It renders no input and sends no request other than that GET.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { fill, locale, setLang } from '../../../shared/i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import { card } from './card';
import * as ratesI18n from './i18n';

type Me = {
  userId: string;
  name: string;
  roles: { role: string; branchId?: string }[];
  branchId: string;
  branchName: string;
  permissions: string[];
};

const BRANCH = 'MAIN';
const PUBLISHED_AT = '2026-10-10T03:15:00Z';

function me(permissions: string[], roles: Me['roles'] = []): Me {
  return {
    userId: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles,
    branchId: BRANCH,
    branchName: 'สาขาทดสอบ',
    permissions,
  };
}

function ctx(m: Me) {
  return { me: m, floor: undefined, refresh: vi.fn(async () => {}) } as unknown as Parameters<
    typeof card.visible
  >[0];
}

/** Rates as the wire carries them: strings, with trailing zeros that a number would lose. */
const RATES: PublishedRatesData = {
  publication: { id: '11111111-2222-4333-8444-555555555555', version: 7, publishedAt: PUBLISHED_AT },
  sourceDate: '2026-10-09',
  stale: false,
  lines: [
    { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '32.4500', sellRate: '32.9000', lineId: 'line-usd' },
    { currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.0450', sellRate: '21.5926', lineId: 'line-jpy' },
    { currency: 'EUR', group: 'ALL', unitSize: '1', buyRate: '37.12000000', sellRate: null, lineId: 'line-eur' },
  ],
};

type Call = { url: string; method: string };
let calls: Call[] = [];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function stubView(data: PublishedRatesData) {
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const method = (init?.method ?? 'GET').toUpperCase();
    calls.push({ url, method });
    if (method === 'GET' && url.startsWith('/api/v3/views/staff_published_rates')) {
      return new Response(JSON.stringify({ version: 3, changed: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: { code: 'notFound', message: 'unexpected request' } }), {
      status: 404,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  vi.stubGlobal('fetch', fetchFn);
}

async function mount(m: Me, data: PublishedRatesData = RATES) {
  stubView(data);
  const C = card.Component;
  const c = ctx(m);
  dispose = render(() => <C ctx={c} />, host);
  await vi.waitFor(() => expect(calls.length).toBeGreaterThan(0));
  // let the answer render
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  return host;
}

beforeEach(() => {
  calls = [];
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

describe('S-8 the card record (SD:§4)', () => {
  it('exports a card with an id, an order, visible(ctx) and a Component', () => {
    expect(typeof card.id).toBe('string');
    expect(card.id.length).toBeGreaterThan(0);
    expect(typeof card.order).toBe('number');
    expect(Number.isFinite(card.order)).toBe(true);
    expect(typeof card.visible).toBe('function');
    expect(typeof card.Component).toBe('function');
  });
});

describe('B11 who sees the card: can(CompatRead), never a role', () => {
  it('is shown to an actor whose permissions hold CompatRead', () => {
    expect(card.visible(ctx(me(['CompatRead'])))).toBe(true);
    expect(card.visible(ctx(me(['AttendanceClock', 'CompatRead', 'ShiftOperate'])))).toBe(true);
  });

  it('is hidden from an actor without CompatRead, whatever else they hold', () => {
    expect(card.visible(ctx(me([])))).toBe(false);
    expect(card.visible(ctx(me(['AttendanceClock', 'ShiftOperate', 'CashManage', 'VerificationReview'])))).toBe(
      false,
    );
  });

  it('reads the permission, not the role names: roles without CompatRead hide it, no roles with it show it', () => {
    // role strings here are deliberately not real role names (the page holds no role literal, SD:S2)
    expect(card.visible(ctx(me([], [{ role: 'ANY_ROLE', branchId: BRANCH }])))).toBe(false);
    expect(card.visible(ctx(me(['CompatRead'], [])))).toBe(true);
  });

  it('spells the permission exactly as staff_me carries it (no case or alias match)', () => {
    expect(card.visible(ctx(me(['compatRead'])))).toBe(false);
    expect(card.visible(ctx(me(['COMPAT_READ'])))).toBe(false);
  });
});

describe('the read: staff_published_rates for the actor\'s branch (S10, rates.md:420)', () => {
  it('reads GET /api/v3/views/staff_published_rates for me.branchId', async () => {
    await mount(me(['CompatRead']));
    const reads = calls.filter((c) => c.url.startsWith('/api/v3/views/staff_published_rates'));
    expect(reads.length).toBeGreaterThan(0);
    for (const r of reads) {
      expect(r.method).toBe('GET');
      const q = new URL(r.url, 'http://x').searchParams;
      expect(q.get('branch')).toBe(BRANCH);
    }
  });

  it('shows every line with its currency and its rates exactly as the wire carries them (strings, not numbers)', async () => {
    const el = await mount(me(['CompatRead']));
    const text = el.textContent ?? '';
    for (const line of RATES.lines) {
      expect(text).toContain(line.currency);
      if (line.buyRate) expect(text).toContain(line.buyRate);
      if (line.sellRate) expect(text).toContain(line.sellRate);
    }
    // a JPY rate is per its unit size: the 100 is shown for that line. Neither JPY rate contains
    // "100", and both are cut out of the row's text first, so only the unit size can satisfy this.
    const jpy = RATES.lines.find((l) => l.currency === 'JPY')!;
    const jpyRow = [...el.querySelectorAll('*')].find(
      (n) => n.children.length > 0 && (n.textContent ?? '').includes('JPY') && !(n.textContent ?? '').includes('USD'),
    );
    const jpyText = (jpyRow?.textContent ?? '')
      .split(jpy.buyRate ?? '\u0000')
      .join(' ')
      .split(jpy.sellRate ?? '\u0000')
      .join(' ');
    expect(jpyText).toContain(jpy.unitSize);
  });

  it('shows a side with no rate as unavailable, never as a made-up figure', async () => {
    const el = await mount(me(['CompatRead']));
    const text = el.textContent ?? '';
    // no boundary: a raw null next to a figure ("37.12000000null") must also be caught
    expect(text).not.toMatch(/null|undefined|NaN/);
    // EUR has no sell rate: its buy rate is shown, nothing invents a sell figure such as 0
    expect(text).toContain('37.12000000');
  });

  it('shows the publication version as given', async () => {
    const el = await mount(me(['CompatRead']));
    expect(el.textContent ?? '').toContain(String(RATES.publication!.version));
  });

  it('with no publication yet shows no rate line and says there is no reference, without failing', async () => {
    const el = await mount(me(['CompatRead']), { publication: null, sourceDate: null, stale: false, lines: [] });
    const text = el.textContent ?? '';
    for (const line of RATES.lines) {
      if (line.buyRate) expect(text).not.toContain(line.buyRate);
    }
    expect(text).toContain('ไม่มีเรทอ้างอิง'); // rate.noSourceDate, through RateFreshness
  });
});

describe('read only: no control writes anything (S10, S5)', () => {
  it('renders no input, select, textarea or form', async () => {
    const el = await mount(me(['CompatRead']));
    expect(el.querySelectorAll('input, select, textarea, form').length).toBe(0);
    expect(el.querySelectorAll('[contenteditable="true"]').length).toBe(0);
  });

  it('sends nothing but GET reads, even after every button in the card is pressed', async () => {
    const el = await mount(me(['CompatRead']));
    for (const b of [...el.querySelectorAll('button')]) {
      (b as HTMLButtonElement).click();
    }
    await new Promise((r) => setTimeout(r, 0));
    for (const c of calls) {
      expect(c.method, c.url).toBe('GET');
      expect(c.url, 'no v2 staff rpc (S5)').not.toContain('/api/staff/rpc');
      expect(c.url, 'only the published-rate view').toMatch(/^\/api\/v3\/views\/staff_published_rates(\?|$)/);
    }
  });
});

describe('freshness: the shared RateFreshness (rates.md §4.10)', () => {
  it('shows the source date and the publication time, and no stale badge when fresh', async () => {
    const el = await mount(me(['CompatRead']));
    const p = el.querySelector('p.xc-rate-freshness');
    expect(p, 'RateFreshness is embedded').not.toBeNull();
    expect(el.querySelector('.xc-rate-stale')).toBeNull();
    const at = new Date(PUBLISHED_AT).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
    expect(p!.textContent).toContain('อ้างอิงวันที่ 2026-10-09');
    expect(p!.textContent).toContain(fill('ประกาศเมื่อ {at}', { at }));
  });

  it('shows the stale badge when the publication is stale', async () => {
    const el = await mount(me(['CompatRead']), { ...RATES, stale: true });
    const badge = el.querySelector('.xc-rate-stale[role="status"]');
    expect(badge).not.toBeNull();
    expect(badge!.textContent).toContain('เรทอ้างอิงเก่า');
  });

  it('never claims the rates are live', async () => {
    const el = await mount(me(['CompatRead']));
    expect((el.textContent ?? '').toLowerCase()).not.toMatch(/real.?time|\blive\b|เรียลไทม์/);
  });
});

describe('TH and EN (SD:S8)', () => {
  it('rates/i18n.ts has th and en tables with the same, non-empty keys', () => {
    const th = (ratesI18n as unknown as { th: Record<string, string> }).th;
    const en = (ratesI18n as unknown as { en: Record<string, string> }).en;
    expect(th && typeof th).toBe('object');
    expect(en && typeof en).toBe('object');
    expect(Object.keys(th).length).toBeGreaterThan(0);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const k of Object.keys(en)) {
      expect(en[k], k).not.toBe('');
      expect(en[k], `en.${k} is not Thai`).not.toMatch(/[฀-๿]/);
    }
  });

  it('in English shows no Thai text at all, and in Thai shows Thai', async () => {
    setLang('en');
    let el = await mount(me(['CompatRead']));
    // the actor's own name and branch are data, not texts; the card is not asked to show them
    const enText = (el.textContent ?? '').replace('คุณทดสอบ', '').replace('สาขาทดสอบ', '');
    expect(enText).not.toMatch(/[฀-๿]/);
    expect(enText).toContain('Reference date 2026-10-09'); // rate.sourceDate in en

    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    calls = [];
    setLang('th');
    el = await mount(me(['CompatRead']));
    expect(el.textContent ?? '').toMatch(/[฀-๿]/);
  });
});

describe('grep rows (SD:S2, §8.1)', () => {
  // Built from pieces so that this file itself holds no role literal and no Number-constructor call (S-3's grep test reads it too).
  const ROLE_WORDS = [
    ['TELL', 'ER'],
    ['BRANCH_MAN', 'AGER'],
    ['RATE_MAN', 'AGER'],
    ['FIN', 'ANCE'],
    ['FRO', 'NT'],
    ['BACK', 'END'],
    ['EXECU', 'TIVE'],
    ['ADM', 'IN'],
  ].map((p) => p.join(''));
  const ROLE_RE = new RegExp(`\\b(${ROLE_WORDS.join('|')})\\b`);
  const NUMBER_CALL = ['Num', 'ber('].join('');

  const sources = import.meta.glob<string>(['./**/*.ts', './**/*.tsx'], {
    query: '?raw',
    import: 'default',
    eager: true,
  });

  it('the card folder has source files besides this test', () => {
    const own = Object.keys(sources).filter((p) => !p.endsWith('rates.test.tsx'));
    expect(own).toContain('./card.ts');
    expect(own).toContain('./i18n.ts');
  });

  it('no role literal in web/src/screens/staff/rates/**', () => {
    for (const [path, src] of Object.entries(sources)) {
      expect(src, path).not.toMatch(ROLE_RE);
    }
  });

  it('no Number-constructor call in web/src/screens/staff/rates/** (money and rates stay strings)', () => {
    for (const [path, src] of Object.entries(sources)) {
      expect(src.includes(NUMBER_CALL), path).toBe(false);
    }
  });
});
