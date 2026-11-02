import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import {
  makeCashLimitsData,
  makeDrawerLowLevel,
  type CashLimitsData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { Toast } from '../parts/toast';
import { DrawerLowLevels } from './DrawerLowLevels';
import * as texts from './i18n';

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
  body: string | null;
  key: string | null;
}

type Reply = { status: number; body?: unknown } | (() => Promise<Response>);

class Server {
  saves: Reply[] = [];
  sent: Sent[] = [];
  private puts = 0;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? init.body : null;
    this.sent.push({
      method,
      path: u.pathname,
      query: u.searchParams,
      body,
      key: headers.get('idempotency-key'),
    });
    if (method === 'PUT' && u.pathname === '/api/cash-limits/drawer-low-levels') {
      const r =
        this.saves[Math.min(this.puts, this.saves.length - 1)] ?? {
          status: 200,
          body: makeCashLimitsData({ branch: 'MAIN' }),
        };
      this.puts++;
      if (typeof r === 'function') return r();
      return json(r.status, r.body ?? {});
    }
    return json(404, {
      error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` },
    });
  };

  writes(): Sent[] {
    return this.sent.filter(
      (s) => s.method === 'PUT' && s.path === '/api/cash-limits/drawer-low-levels',
    );
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;
let onSavedCalls: CashLimitsData[] = [];

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

const text = (el: Element | null | undefined) =>
  (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const screen = (el: ParentNode) =>
  el.querySelector<HTMLElement>('[data-section="drawer-low-levels"]');
const rowElements = (el: ParentNode) =>
  el.querySelectorAll<HTMLElement>('[data-level-row]');
const addButton = (el: ParentNode) =>
  el.querySelector<HTMLButtonElement>('button[data-action="add-level"]');
const saveLevelsButton = (el: ParentNode) =>
  el.querySelector<HTMLButtonElement>('button[data-action="save-levels"]');

function typeInput(el: HTMLInputElement | null, value: string): void {
  expect(el, 'input exists').not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

async function mount(opts: {
  data?: CashLimitsData | null;
  branch?: string;
  saves?: Reply[];
} = {}) {
  if (opts.saves) server.saves = opts.saves;
  const [data, setData] = createSignal<CashLimitsData | null>(
    opts.data !== undefined
      ? opts.data
      : makeCashLimitsData({
          branch: 'MAIN',
          drawerLowLevels: [],
        }),
  );
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');

  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <DrawerLowLevels
            data={data}
            branch={branch}
            onSaved={(d) => {
              onSavedCalls.push(d);
            }}
          />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return { root, data, setData, branch, setBranch };
}

beforeEach(() => {
  server = new Server();
  onSavedCalls = [];
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  setLang('th');
});

afterEach(async () => {
  await settle();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('DrawerLowLevels (§5.4d, #5980)', () => {
  it('C1: the view answers levels: one row per level with currency (read-only) and level (input) in answer order', async () => {
    const initial = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [
        makeDrawerLowLevel({ currency: 'USD', lowLevel: '1000.00' }),
        makeDrawerLowLevel({ currency: 'EUR', lowLevel: '500.00' }),
      ],
    });
    const { root } = await mount({ data: initial });
    const s = screen(root)!;
    const rows = rowElements(s);
    expect(rows).toHaveLength(2);

    // Row 0: USD in answer order, currency is read-only (no currency input)
    expect(rows[0]!.querySelector('input[name="currency"]')).toBeNull();
    expect(rows[0]!.querySelector('[data-field="currency"]')?.textContent).toBe('USD');
    expect(rows[0]!.querySelector<HTMLInputElement>('input[name="lowLevel"]')?.value).toBe(
      '1000.00',
    );

    // Row 1: EUR in answer order
    expect(rows[1]!.querySelector('input[name="currency"]')).toBeNull();
    expect(rows[1]!.querySelector('[data-field="currency"]')?.textContent).toBe('EUR');
    expect(rows[1]!.querySelector<HTMLInputElement>('input[name="lowLevel"]')?.value).toBe(
      '500.00',
    );
  });

  it('C2: drawerLowLevels: [] shows levelsEmpty and the add button', async () => {
    const initial = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [],
    });
    const { root } = await mount({ data: initial });
    const s = screen(root)!;
    expect(text(s)).toContain(texts.th.levelsEmpty);
    expect(addButton(s)).not.toBeNull();
    expect(rowElements(s)).toHaveLength(0);
  });

  it('C3: add shows a row with a currency input (3 letters, upper-cased as typed) and a level input', async () => {
    const initial = makeCashLimitsData({ branch: 'MAIN', drawerLowLevels: [] });
    const { root } = await mount({ data: initial });
    const s = screen(root)!;
    addButton(s)!.click();
    await settle();

    const rows = rowElements(s);
    expect(rows).toHaveLength(1);
    const curInput = rows[0]!.querySelector<HTMLInputElement>('input[name="currency"]');
    const lvlInput = rows[0]!.querySelector<HTMLInputElement>('input[name="lowLevel"]');
    expect(curInput).not.toBeNull();
    expect(lvlInput).not.toBeNull();

    // Type lowercase 'usd' -> becomes 'USD'
    typeInput(curInput, 'usd');
    expect(curInput?.value).toBe('USD');

    // Clamped to 3 letters
    typeInput(curInput, 'USDT');
    expect(curInput?.value).toBe('USD');

    typeInput(lvlInput, '200.00');
    expect(lvlInput?.value).toBe('200.00');
  });

  it('C4: remove leaves the form; nothing is sent until save', async () => {
    const initial = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [
        makeDrawerLowLevel({ currency: 'USD', lowLevel: '1000.00' }),
        makeDrawerLowLevel({ currency: 'EUR', lowLevel: '500.00' }),
      ],
    });
    const { root } = await mount({ data: initial });
    const s = screen(root)!;
    expect(rowElements(s)).toHaveLength(2);

    const removeBtn = rowElements(s)[0]!.querySelector<HTMLButtonElement>(
      'button[data-action="remove-level"]',
    );
    removeBtn!.click();
    await settle();

    expect(rowElements(s)).toHaveLength(1);
    expect(rowElements(s)[0]!.querySelector('[data-field="currency"]')?.textContent).toBe('EUR');
    expect(server.writes()).toHaveLength(0);
  });

  it('C5: save sends exactly one keyed PUT with {branch, levels} in form order trimmed, calls onSaved, toasts levelsSaved', async () => {
    const initial = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [makeDrawerLowLevel({ currency: 'USD', lowLevel: '1000.00' })],
    });
    const savedResult = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [
        makeDrawerLowLevel({ currency: 'USD', lowLevel: '1200.00' }),
        makeDrawerLowLevel({ currency: 'JPY', lowLevel: '50000.00' }),
      ],
    });
    const { root } = await mount({
      data: initial,
      saves: [{ status: 200, body: savedResult }],
    });
    const s = screen(root)!;

    // Edit USD level with extra whitespace
    const usdLevel = rowElements(s)[0]!.querySelector<HTMLInputElement>(
      'input[name="lowLevel"]',
    );
    typeInput(usdLevel, '  1200.00  ');

    // Add JPY with whitespace
    addButton(s)!.click();
    await settle();
    const jpyRow = rowElements(s)[1]!;
    const jpyCur = jpyRow.querySelector<HTMLInputElement>('input[name="currency"]');
    const jpyLvl = jpyRow.querySelector<HTMLInputElement>('input[name="lowLevel"]');
    typeInput(jpyCur, 'jpy');
    typeInput(jpyLvl, ' 50000.00 ');

    saveLevelsButton(s)!.click();
    await settle();

    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.method).toBe('PUT');
    expect(w[0]!.path).toBe('/api/cash-limits/drawer-low-levels');
    expect(w[0]!.key).toBeTruthy();
    expect(JSON.parse(w[0]!.body ?? '{}')).toEqual({
      branch: 'MAIN',
      levels: [
        { currency: 'USD', lowLevel: '1200.00' },
        { currency: 'JPY', lowLevel: '50000.00' },
      ],
    });
    expect(onSavedCalls).toHaveLength(1);
    expect(onSavedCalls[0]).toEqual(savedResult);
    expect(text(document.body)).toContain(texts.th.levelsSaved);
  });

  it('C6: 400 validationFailed shows refusal under the levels form and what was typed is kept', async () => {
    const initial = makeCashLimitsData({ branch: 'MAIN', drawerLowLevels: [] });
    const { root } = await mount({
      data: initial,
      saves: [
        {
          status: 400,
          body: {
            error: {
              code: 'validationFailed',
              field: 'levels',
              message: 'ข้อมูลไม่ถูกต้อง',
            },
          },
        },
      ],
    });
    const s = screen(root)!;
    addButton(s)!.click();
    await settle();

    const curInput = rowElements(s)[0]!.querySelector<HTMLInputElement>(
      'input[name="currency"]',
    );
    const lvlInput = rowElements(s)[0]!.querySelector<HTMLInputElement>(
      'input[name="lowLevel"]',
    );
    typeInput(curInput, 'xxx');
    typeInput(lvlInput, 'invalid');

    saveLevelsButton(s)!.click();
    await settle();

    const errEl = s.querySelector('[data-form-error]');
    expect(errEl).not.toBeNull();
    expect(text(errEl)).toBeTruthy();
    expect(curInput?.value).toBe('XXX');
    expect(lvlInput?.value).toBe('invalid');
  });

  it('C8: save with no rows sends levels: []', async () => {
    const initial = makeCashLimitsData({ branch: 'MAIN', drawerLowLevels: [] });
    const savedResult = makeCashLimitsData({ branch: 'MAIN', drawerLowLevels: [] });
    const { root } = await mount({
      data: initial,
      saves: [{ status: 200, body: savedResult }],
    });
    const s = screen(root)!;

    saveLevelsButton(s)!.click();
    await settle();

    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(JSON.parse(w[0]!.body ?? '{}')).toEqual({
      branch: 'MAIN',
      levels: [],
    });
    expect(onSavedCalls).toHaveLength(1);
    expect(onSavedCalls[0]).toEqual(savedResult);
  });

  it('edge case 1: the branch changes while form has unsaved edits: follows data', async () => {
    const mainData = makeCashLimitsData({
      branch: 'MAIN',
      drawerLowLevels: [makeDrawerLowLevel({ currency: 'USD', lowLevel: '100.00' })],
    });
    const b1Data = makeCashLimitsData({
      branch: 'B1',
      drawerLowLevels: [makeDrawerLowLevel({ currency: 'EUR', lowLevel: '200.00' })],
    });
    const { root, setData, setBranch } = await mount({ data: mainData, branch: 'MAIN' });
    const s = screen(root)!;

    typeInput(
      rowElements(s)[0]!.querySelector<HTMLInputElement>('input[name="lowLevel"]'),
      '999.00',
    );

    setBranch('B1');
    setData(b1Data);
    await settle();

    expect(rowElements(s)).toHaveLength(1);
    expect(rowElements(s)[0]!.querySelector('[data-field="currency"]')?.textContent).toBe('EUR');
    expect(
      rowElements(s)[0]!.querySelector<HTMLInputElement>('input[name="lowLevel"]')?.value,
    ).toBe('200.00');
  });

  it('edge case 2: data is null: no levels form', async () => {
    const { root } = await mount({ data: null });
    expect(screen(root)).toBeNull();
  });
});
