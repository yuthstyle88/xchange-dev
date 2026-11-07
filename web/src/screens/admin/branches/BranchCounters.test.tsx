// XC Counter CA-2 (docs/design/xc-counter-app.md §4, §5.2, §5.4, §8.4): Admin → สาขา → เคาน์เตอร์.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//
// | # | Case | Required |
// |---|---|---|
// | R1 | the view answers | reads `branch_counters?branch=`; one row per counter with its name and last seen |
// | R2 | a counter never seen | "ยังไม่เคยใช้งาน" |
// | R3 | no counters | "ยังไม่มีเคาน์เตอร์ของสาขานี้" |
// | R4 | a null setting | shows "ค่าเริ่มต้น", meaning drawer off, scanner off, kiosk lock on |
// | R5 | a set setting | shows its value |
// | R6 | set a toggle | `branchCounterSave` with the full row (all three settings + name) |
// | R7 | choose "ค่าเริ่มต้น" | sends null for that setting |
// | R8 | rename | `branchCounterSave` with the new name, settings unchanged |
// | R9 | the save's answer | replaces the row; the next save sends it |
// | R10 | a refused save | `wordedError` in `role="alert"` |
// | R11 | a null branch toggle | "ค่าเริ่มต้น (เปิด)" |
// | R12 | set the branch toggle | `branchCashFromBrowserSave {branch, expectedVersion, value}`; the next save uses the answered version; default sends null |
// | R13 | a version conflict | the conflict message, and the view is read again |
// | R14 | another branch | reads that branch's view |
// | R15 | BranchesScreen, `branchSettingsSave` held | the panel mounts |
// | R16 | BranchesScreen, not held | no panel, no read of the view |
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { sharedSession } from '../../../shared/session/session';
import { makeAdminBranchItem, makeAdminBranchesData } from '../../../shared/testing/fixtures/admin_branches';
import { makeAdminBranchProfileData } from '../../../shared/testing/fixtures/admin_branches_profile';
import { makeBranchCounterRow } from '../../../shared/testing/fixtures/branch_counters';
import { makeMe } from '../../../shared/testing/fixtures/common';
import { BranchCounters } from './BranchCounters';
import { BranchesScreen } from './BranchesScreen';
import type { BranchCounterRow, BranchCountersData } from './counters-api';

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const COUNTER_SAVE = '/api/v3/commands/branch_counter_save';
const CASH_SAVE = '/api/v3/commands/branch_cash_from_browser_save';
const VIEW = '/api/v3/views/branch_counters';

const NAME = 'ชื่อเคาน์เตอร์';
const DRAWER = 'รองรับลิ้นชักเงิน';
const SCANNER = 'มีเครื่องสแกนพาสปอร์ต';
const LOCK = 'ล็อกเต็มจอ';

const SEEN = makeBranchCounterRow({
  agentId: 'agent-1',
  name: 'เคาน์เตอร์ 1',
  pairedAt: '2026-11-01T02:00:00Z',
  lastSeenAt: '2026-11-06T02:00:00Z',
  cashDrawer: null,
  scanner: null,
  kioskLock: null,
});
const SET = makeBranchCounterRow({
  agentId: 'agent-2',
  name: 'เคาน์เตอร์ 2',
  pairedAt: '2026-11-02T02:00:00Z',
  lastSeenAt: null,
  cashDrawer: true,
  scanner: false,
  kioskLock: false,
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

interface Call {
  url: string;
  method: string;
  body: unknown;
  key: string | null;
}

let calls: Call[];
let world: BranchCountersData;
let answerSave: (url: string, body: unknown) => Response;

const fetchFn: Fetch = async (input, init) => {
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url: input, method, body, key: new Headers(init?.headers).get('idempotency-key') });
  const u = new URL(input, 'http://localhost');
  if (u.pathname === VIEW) return json({ version: 1, changed: true, data: world });
  return answerSave(u.pathname, body);
};

const writes = (path: string) => calls.filter((c) => c.url === path && c.method === 'POST');
const reads = () => calls.filter((c) => c.url.startsWith(VIEW));

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  calls = [];
  world = { counters: [SEEN, SET], cashFromBrowser: null, cashFromBrowserVersion: 0 };
  answerSave = (url, body) => {
    if (url === COUNTER_SAVE) {
      const b = body as Omit<BranchCounterRow, 'pairedAt' | 'lastSeenAt'>;
      return json({
        counter: { ...world.counters.find((c) => c.agentId === b.agentId)!, ...b },
      });
    }
    if (url === CASH_SAVE) {
      const b = body as { value: boolean | null; expectedVersion: number };
      return json({ value: b.value, version: b.expectedVersion + 1 });
    }
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle() {
  for (let i = 0; i < 20; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

async function mount(branch = B01) {
  const [b, setBranch] = createSignal(branch);
  dispose = render(() => <BranchCounters branchId={b()} api={new ApiClient(fetchFn)} />, host);
  await settle();
  return { setBranch };
}

function panel(): HTMLElement {
  const p = host.querySelector<HTMLElement>('[data-counters-panel]');
  expect(p, 'the counters panel').not.toBeNull();
  return p!;
}

function row(agentId: string): HTMLElement {
  const r = host.querySelector<HTMLElement>(`[data-counter-row="${agentId}"]`);
  expect(r, `the row of ${agentId}`).not.toBeNull();
  return r!;
}

function labelled(r: HTMLElement, text: string): HTMLLabelElement {
  const l = [...r.querySelectorAll('label')].find((x) => (x.textContent ?? '').includes(text));
  expect(l, `the label "${text}"`).toBeDefined();
  return l!;
}

function setting(agentId: string, text: string): HTMLSelectElement {
  const s = labelled(row(agentId), text).querySelector('select');
  expect(s, `the select of "${text}"`).not.toBeNull();
  return s!;
}

function shown(select: HTMLSelectElement): string {
  return select.selectedOptions[0]?.textContent?.trim() ?? '';
}

function branchToggle(): HTMLSelectElement {
  const s = host.querySelector<HTMLSelectElement>('[data-cash-from-browser] select');
  expect(s, 'the branch toggle').not.toBeNull();
  return s!;
}

async function choose(select: HTMLSelectElement, value: '' | 'true' | 'false') {
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
}

function alertText(): string {
  return [...host.querySelectorAll('[role="alert"]')].map((a) => a.textContent ?? '').join(' ');
}

describe('BranchCounters: the list', () => {
  it("R1: reads branch_counters for the branch and lists each counter's name and last seen", async () => {
    await mount();
    expect(reads().map((c) => new URL(c.url, 'http://x').searchParams.get('branch'))).toEqual([B01]);
    expect(panel().textContent).toContain('เคาน์เตอร์');
    expect(labelled(row(SEEN.agentId), NAME).querySelector('input')?.value).toBe(SEEN.name);
    expect(labelled(row(SET.agentId), NAME).querySelector('input')?.value).toBe(SET.name);
    expect(row(SEEN.agentId).textContent).toContain('ใช้งานล่าสุด');
    expect(row(SEEN.agentId).textContent).toContain(SEEN.lastSeenAt);
  });

  it('R2: a counter never seen says so', async () => {
    await mount();
    expect(row(SET.agentId).textContent).toContain('ยังไม่เคยใช้งาน');
  });

  it('R3: no counters says so', async () => {
    world = { counters: [], cashFromBrowser: null, cashFromBrowserVersion: 0 };
    await mount();
    expect(panel().textContent).toContain('ยังไม่มีเคาน์เตอร์ของสาขานี้');
  });

  it('R4: a null setting shows ค่าเริ่มต้น with its meaning: drawer off, scanner off, kiosk lock on', async () => {
    await mount();
    expect(shown(setting(SEEN.agentId, DRAWER))).toBe('ค่าเริ่มต้น (ปิด)');
    expect(shown(setting(SEEN.agentId, SCANNER))).toBe('ค่าเริ่มต้น (ปิด)');
    expect(shown(setting(SEEN.agentId, LOCK))).toBe('ค่าเริ่มต้น (เปิด)');
  });

  it('R5: a set setting shows its value', async () => {
    await mount();
    expect(shown(setting(SET.agentId, DRAWER))).toBe('เปิด');
    expect(shown(setting(SET.agentId, SCANNER))).toBe('ปิด');
    expect(shown(setting(SET.agentId, LOCK))).toBe('ปิด');
  });
});

describe('BranchCounters: saving a counter', () => {
  it('R6: setting a toggle sends branchCounterSave with the full row', async () => {
    await mount();
    await choose(setting(SEEN.agentId, SCANNER), 'true');
    const sent = writes(COUNTER_SAVE);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.key, 'sent through keyedWrite').toBeTruthy();
    expect(sent[0]!.body).toEqual({
      branch: B01,
      agentId: SEEN.agentId,
      name: SEEN.name,
      cashDrawer: null,
      scanner: true,
      kioskLock: null,
    });
  });

  it('R7: choosing ค่าเริ่มต้น sends null for that setting', async () => {
    await mount();
    await choose(setting(SET.agentId, DRAWER), '');
    expect(writes(COUNTER_SAVE)[0]!.body).toEqual({
      branch: B01,
      agentId: SET.agentId,
      name: SET.name,
      cashDrawer: null,
      scanner: false,
      kioskLock: false,
    });
  });

  it('R8: a rename sends the new name and the settings unchanged', async () => {
    await mount();
    const input = labelled(row(SET.agentId), NAME).querySelector('input')!;
    input.value = 'เคาน์เตอร์ใหม่';
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(writes(COUNTER_SAVE)[0]!.body).toEqual({
      branch: B01,
      agentId: SET.agentId,
      name: 'เคาน์เตอร์ใหม่',
      cashDrawer: true,
      scanner: false,
      kioskLock: false,
    });
  });

  it("R9: the save's answer replaces the row, and the next save sends it", async () => {
    await mount();
    answerSave = () => json({ counter: { ...SEEN, name: 'ชื่อจากเซิร์ฟเวอร์', kioskLock: false } });
    await choose(setting(SEEN.agentId, DRAWER), 'true');
    expect(labelled(row(SEEN.agentId), NAME).querySelector('input')?.value).toBe('ชื่อจากเซิร์ฟเวอร์');
    expect(shown(setting(SEEN.agentId, LOCK))).toBe('ปิด');

    answerSave = () => json({ counter: SEEN });
    await choose(setting(SEEN.agentId, SCANNER), 'true');
    expect(writes(COUNTER_SAVE)[1]!.body).toMatchObject({ name: 'ชื่อจากเซิร์ฟเวอร์', kioskLock: false, scanner: true });
  });

  it('R10: a refused save is worded in role="alert"', async () => {
    await mount();
    answerSave = () => json({ error: { code: 'ownerOnly', message: 'เฉพาะเจ้าของระบบเท่านั้น' } }, 403);
    await choose(setting(SEEN.agentId, DRAWER), 'true');
    expect(alertText()).not.toBe('');
  });
});

describe('BranchCounters: the branch toggle', () => {
  it('R11: null shows ค่าเริ่มต้น (เปิด)', async () => {
    await mount();
    expect(shown(branchToggle())).toBe('ค่าเริ่มต้น (เปิด)');
    expect(branchToggle().closest('label')?.textContent).toContain('จ่ายเงินสดจากเบราว์เซอร์ได้');
  });

  it('R12: saves through branchCashFromBrowserSave with expectedVersion; the next save uses the answered version', async () => {
    world = { ...world, cashFromBrowser: true, cashFromBrowserVersion: 4 };
    await mount();
    expect(shown(branchToggle())).toBe('เปิด');
    await choose(branchToggle(), 'false');
    await choose(branchToggle(), '');
    expect(writes(CASH_SAVE).map((c) => c.body)).toEqual([
      { branch: B01, expectedVersion: 4, value: false },
      { branch: B01, expectedVersion: 5, value: null },
    ]);
    expect(writes(CASH_SAVE)[0]!.key, 'sent through keyedWrite').toBeTruthy();
    expect(writes(COUNTER_SAVE)).toHaveLength(0);
  });

  it('R13: a version conflict shows the conflict message and reads the view again', async () => {
    await mount();
    const before = reads().length;
    answerSave = () => json({ error: { code: 'staleVersion', message: 'stale version' } }, 409);
    world = { ...world, cashFromBrowser: false, cashFromBrowserVersion: 9 };
    await choose(branchToggle(), 'true');
    expect(alertText()).toContain(sharedTh['error.staleVersion']);
    expect(reads().length, 'the view is read again').toBe(before + 1);
    expect(shown(branchToggle()), "shows the server's value").toBe('ปิด');
    answerSave = () => json({ value: true, version: 10 });
    await choose(branchToggle(), 'true');
    expect(writes(CASH_SAVE).at(-1)!.body).toEqual({ branch: B01, expectedVersion: 9, value: true });
  });

  it("R14: another branch reads that branch's view", async () => {
    const { setBranch } = await mount();
    setBranch(B02);
    await settle();
    expect(reads().map((c) => new URL(c.url, 'http://x').searchParams.get('branch'))).toEqual([B01, B02]);
    await choose(branchToggle(), 'false');
    expect(writes(CASH_SAVE)[0]!.body).toMatchObject({ branch: B02 });
  });
});

describe('BranchesScreen mounts the counters for the owner only', () => {
  function serve() {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push({ url, method: (init?.method ?? 'GET').toUpperCase(), body: undefined, key: null });
        const u = new URL(url, 'http://localhost');
        if (u.pathname === '/api/v3/views/admin_branches') {
          return json({
            version: 1,
            changed: true,
            data: makeAdminBranchesData({ branches: [makeAdminBranchItem({ id: B01, name: 'Silom Branch' })] }),
          });
        }
        if (u.pathname === '/api/v3/views/admin_branches_profile') {
          return json({
            version: 1,
            changed: true,
            data: makeAdminBranchProfileData({ branch: B01, name: 'Silom Branch', code: 'SILOM', address: '1 Silom Rd', timezone: 'Asia/Bangkok', bookingEnabled: true }),
          });
        }
        if (u.pathname === VIEW) return json({ version: 1, changed: true, data: world });
        if (u.pathname === '/api/branch-tablets') return json({ tablets: [] });
        return json({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  }

  function mountScreen() {
    const [branch] = createSignal(B01);
    const [on] = createSignal(true);
    dispose = render(
      () => <BranchesScreen branch={branch} period={() => ({ from: '2026-11-01', to: '2026-11-30' })} shown={on} />,
      host,
    );
  }

  it('R15: with branchSettingsSave in me.permissions, the panel mounts and reads the view', async () => {
    serve();
    sharedSession().updateMe(makeMe({ permissions: ['BranchesManage', 'branchSettingsSave'] }));
    mountScreen();
    await vi.waitFor(() => {
      expect(host.querySelector('[data-counters-panel]'), 'the counters panel').not.toBeNull();
    });
    expect(reads().map((c) => new URL(c.url, 'http://x').searchParams.get('branch'))).toEqual([B01]);
  });

  it('R16: without branchSettingsSave, even with SystemSettings, no panel and no read of the view', async () => {
    serve();
    sharedSession().updateMe(makeMe({ permissions: ['BranchesManage', 'SystemSettings'] }));
    mountScreen();
    await vi.waitFor(() => {
      expect(calls.some((c) => c.url.includes('admin_branches_profile')), 'the profile was read').toBe(true);
    });
    await settle();
    expect(host.querySelector('[data-counters-panel]')).toBeNull();
    expect(reads()).toEqual([]);
  });
});
