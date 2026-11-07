// XC Counter CA-2b (#6405): follow-up tests for BranchCounters pending saves, refusal reset, and first-load state.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { makeBranchCounterRow } from '../../../shared/testing/fixtures/branch_counters';
import { BranchCounters } from './BranchCounters';
import type { BranchCounterRow, BranchCountersData } from './counters-api';

const B01 = 'XC-B01';
const COUNTER_SAVE = '/api/v3/commands/branch_counter_save';
const CASH_SAVE = '/api/v3/commands/branch_cash_from_browser_save';
const VIEW = '/api/v3/views/branch_counters';

const NAME = 'ชื่อเคาน์เตอร์';
const DRAWER = 'รองรับลิ้นชักเงิน';
const SCANNER = 'มีเครื่องสแกนพาสปอร์ต';
const LOCK = 'ล็อกเต็มจอ';
const EMPTY = 'ยังไม่มีเคาน์เตอร์ของสาขานี้';

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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

interface Call {
  url: string;
  method: string;
  body: unknown;
}

let calls: Call[];
let world: BranchCountersData;
let answerView: () => Response | Promise<Response>;
let answerSave: (url: string, body: unknown) => Response | Promise<Response>;

const fetchFn: Fetch = async (input, init) => {
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url: input, method, body });
  const u = new URL(input, 'http://localhost');
  if (u.pathname === VIEW) return answerView();
  return answerSave(u.pathname, body);
};

const writes = (path: string) => calls.filter((c) => c.url === path && c.method === 'POST');

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  calls = [];
  world = { counters: [SEEN, SET], cashFromBrowser: null, cashFromBrowserVersion: 0 };
  answerView = () => json({ version: 1, changed: true, data: world });
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

function nameInput(agentId: string): HTMLInputElement {
  const i = labelled(row(agentId), NAME).querySelector('input');
  expect(i, 'the name input').not.toBeNull();
  return i!;
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

describe('BranchCounters CA-2b follow-ups (#6405)', () => {
  it('(a) a second change while the first save is pending cannot be sent: the control is disabled; after the answer, the next save carries both changes', async () => {
    await mount();
    const firstCounterSave = deferred<Response>();
    answerSave = () => firstCounterSave.promise;

    await choose(setting(SEEN.agentId, DRAWER), 'true');
    expect(writes(COUNTER_SAVE)).toHaveLength(1);
    expect(nameInput(SEEN.agentId).disabled).toBe(true);
    expect(setting(SEEN.agentId, DRAWER).disabled).toBe(true);
    expect(setting(SEEN.agentId, SCANNER).disabled).toBe(true);
    expect(setting(SEEN.agentId, LOCK).disabled).toBe(true);
    expect(setting(SET.agentId, DRAWER).disabled).toBe(false);

    firstCounterSave.resolve(json({ counter: { ...SEEN, cashDrawer: true } }));
    await settle();

    expect(nameInput(SEEN.agentId).disabled).toBe(false);
    expect(setting(SEEN.agentId, DRAWER).disabled).toBe(false);
    expect(setting(SEEN.agentId, SCANNER).disabled).toBe(false);
    expect(setting(SEEN.agentId, LOCK).disabled).toBe(false);

    answerSave = (_url, body) => {
      const b = body as Omit<BranchCounterRow, 'pairedAt' | 'lastSeenAt'>;
      return json({ counter: { ...SEEN, ...b } });
    };
    await choose(setting(SEEN.agentId, SCANNER), 'true');
    expect(writes(COUNTER_SAVE)).toHaveLength(2);
    expect(writes(COUNTER_SAVE)[1]!.body).toEqual({
      branch: B01,
      agentId: SEEN.agentId,
      name: SEEN.name,
      cashDrawer: true,
      scanner: true,
      kioskLock: null,
    });

    const firstBranchSave = deferred<Response>();
    answerSave = () => firstBranchSave.promise;
    await choose(branchToggle(), 'false');
    expect(writes(CASH_SAVE)).toHaveLength(1);
    expect(branchToggle().disabled).toBe(true);

    firstBranchSave.resolve(json({ value: false, version: 1 }));
    await settle();
    expect(branchToggle().disabled).toBe(false);
  });

  it('(b) a refused counter save, and a refused branch save, put the select back to the stored value', async () => {
    await mount();
    answerSave = () => json({ error: { code: 'ownerOnly', message: 'เฉพาะเจ้าของระบบเท่านั้น' } }, 403);

    await choose(setting(SEEN.agentId, DRAWER), 'true');
    expect(setting(SEEN.agentId, DRAWER).value).toBe('');
    expect(shown(setting(SEEN.agentId, DRAWER))).toBe('ค่าเริ่มต้น (ปิด)');

    await choose(branchToggle(), 'false');
    expect(branchToggle().value).toBe('');
    expect(shown(branchToggle())).toBe('ค่าเริ่มต้น (เปิด)');
  });

  it('(c) a staleVersion whose re-read is unchanged puts the select back', async () => {
    world = { ...world, cashFromBrowser: true, cashFromBrowserVersion: 2 };
    await mount();
    expect(shown(branchToggle())).toBe('เปิด');

    answerSave = () => json({ error: { code: 'staleVersion', message: 'stale version' } }, 409);
    await choose(branchToggle(), 'false');
    expect(alertText()).toContain(sharedTh['error.staleVersion']);
    expect(branchToggle().value).toBe('true');
    expect(shown(branchToggle())).toBe('เปิด');
  });

  it('(d) a refused rename puts the stored name back', async () => {
    await mount();
    answerSave = () => json({ error: { code: 'ownerOnly', message: 'เฉพาะเจ้าของระบบเท่านั้น' } }, 403);

    const input = nameInput(SET.agentId);
    input.value = 'ชื่อที่ถูกปฏิเสธ';
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(input.value).toBe(SET.name);
  });

  it('(e) a failed first read shows the alert and no empty-list text', async () => {
    answerView = () => json({ error: { code: 'serverError', message: 'Internal error' } }, 500);
    await mount();
    expect(alertText()).toContain(sharedTh['error.serverError']);
    expect(panel().textContent).not.toContain(EMPTY);
  });

  it('(f) before the first answer, there is no empty-list text', async () => {
    const viewGate = deferred<Response>();
    answerView = () => viewGate.promise;
    await mount();
    expect(panel().textContent).not.toContain(EMPTY);

    viewGate.resolve(json({ version: 1, changed: true, data: { counters: [], cashFromBrowser: null, cashFromBrowserVersion: 0 } }));
    await settle();
    expect(panel().textContent).toContain(EMPTY);
  });
});
