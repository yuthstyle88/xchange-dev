import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { StaffCtx } from '../cards';
import {
  makeCountSubmitInput,
  makeDenominationRow,
  makeHandoverReceiverRow,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  type CountSubmitInput,
  type StaffFloorView,
  type StaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor(over: Partial<StaffFloorView> = {}) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: makeStaffFloorMyShift({
      id: 'shift-open-001',
      openedAt: '2026-10-11T01:05:00Z',
    }),
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  });
}

const SAMPLE_COUNTS: never[] = [];
const SAMPLE_HANDOVERS: never[] = [];
const SAMPLE_DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '500.0000', groupCode: 'NOTE' }),
];
const SAMPLE_RECEIVERS = [
  makeHandoverReceiverRow({ userId: OTHER, name: 'Somchai' }),
];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Staff shift count: empty denominations guard (#2239)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let viewHandlers: Record<string, () => Promise<Response>>;
  let cashCountPosts: CountSubmitInput[];

  function stubViews(overrides: Record<string, () => Promise<Response>> = {}) {
    viewHandlers = {
      cash_counts: async () => json(200, { version: 1, changed: true, data: SAMPLE_COUNTS }),
      handovers_pending: async () => json(200, { version: 1, changed: true, data: SAMPLE_HANDOVERS }),
      denominations: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
      handover_receivers: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } }),
      ...overrides,
    };

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url =
          typeof input === 'string'
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        const path = url.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && path.startsWith('/api/v3/views/')) {
          const viewName = path.slice('/api/v3/views/'.length);
          if (viewHandlers[viewName]) {
            return viewHandlers[viewName]();
          }
          return json(404, { error: { code: 'notFound', message: 'not found' } });
        }
        if (method === 'POST' && path === '/api/cash-counts') {
          const raw = typeof init?.body === 'string' ? init.body : '{}';
          cashCountPosts.push(makeCountSubmitInput(JSON.parse(raw)));
          return json(200, { id: 'count-new-001' });
        }
        return json(200, {});
      }),
    );
  }

  async function settle() {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  }

  function mount(m: StaffMeView = me(['ShiftOperate']), f: StaffFloorView = floor()) {
    const ctx: StaffCtx = { me: m, floor: f, refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    return { ctx };
  }

  function tid(id: string): string {
    return `[data-testid="${id}"]`;
  }

  function q<T extends Element = HTMLElement>(sel: string): T | null {
    return host.querySelector<T>(sel);
  }

  function must<T extends Element = HTMLElement>(sel: string): T {
    const el = q<T>(sel);
    expect(el, sel).not.toBeNull();
    return el!;
  }

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    cashCountPosts = [];
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('row 1: Denominations read fails, count form open, press Save -> No POST /api/cash-counts; Save disabled; the load error stays shown', async () => {
    stubViews({
      denominations: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount();
    await vi.waitFor(() => {
      expect(q(tid('shift-error'))?.textContent?.trim()).toBe(sharedTh['error.serverError']);
    });

    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await settle();

    const saveBtn = must<HTMLButtonElement>(tid('shift-count-save'));
    expect(saveBtn.disabled).toBe(true);
    expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(0);

    saveBtn.click();
    must<HTMLFormElement>(tid('shift-count-form')).dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    );
    await settle();

    expect(cashCountPosts).toEqual([]);
    expect(saveBtn.disabled).toBe(true);
    expect(q(tid('shift-error'))?.textContent?.trim()).toBe(sharedTh['error.serverError']);
  });

  it('row 2: Denominations read fails, then the re-read succeeds -> Inputs appear; Save enabled; a filled count posts its lines', async () => {
    let denomCalls = 0;
    stubViews({
      denominations: async () => {
        denomCalls += 1;
        if (denomCalls <= 2) {
          return json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });
        }
        return json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });
      },
    });

    mount();
    await vi.waitFor(() => {
      expect(q(tid('shift-error'))?.textContent?.trim()).toBe(sharedTh['error.serverError']);
    });

    // Open count while denominations read still fails (call 2): Save is disabled and no inputs
    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await settle();
    expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(0);
    expect(must<HTMLButtonElement>(tid('shift-count-save')).disabled).toBe(true);

    // Re-open count to trigger denominations re-read (call 3, which succeeds)
    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await vi.waitFor(() => {
      expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(2);
    });

    const saveBtn = must<HTMLButtonElement>(tid('shift-count-save'));
    expect(saveBtn.disabled).toBe(false);

    const input11 = must<HTMLInputElement>('input[data-denomination-id="11"]');
    input11.value = '5';
    input11.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    await vi.waitFor(() => {
      expect(cashCountPosts.length).toBe(1);
    });
    expect(cashCountPosts[0]?.lines).toEqual([
      { currency: 'THB', denominationId: 11, qty: '5' },
    ]);
  });

  it('row 3: Denominations loaded, quantities entered, Save -> POST with those lines (kept)', async () => {
    stubViews();
    mount();
    await settle();

    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await vi.waitFor(() => {
      expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(2);
    });

    const input11 = must<HTMLInputElement>('input[data-denomination-id="11"]');
    const input12 = must<HTMLInputElement>('input[data-denomination-id="12"]');
    input11.value = '3';
    input11.dispatchEvent(new Event('input', { bubbles: true }));
    input12.value = '7';
    input12.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = must<HTMLButtonElement>(tid('shift-count-save'));
    expect(saveBtn.disabled).toBe(false);
    saveBtn.click();

    await vi.waitFor(() => {
      expect(cashCountPosts.length).toBe(1);
    });
    expect(cashCountPosts[0]?.lines).toEqual([
      { currency: 'THB', denominationId: 11, qty: '3' },
      { currency: 'THB', denominationId: 12, qty: '7' },
    ]);
  });

  it('row 4: Denominations loaded, a quantity of 0 entered, Save -> POST with qty "0" for that line', async () => {
    stubViews();
    mount();
    await settle();

    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await vi.waitFor(() => {
      expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(2);
    });

    const input11 = must<HTMLInputElement>('input[data-denomination-id="11"]');
    input11.value = '0';
    input11.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = must<HTMLButtonElement>(tid('shift-count-save'));
    expect(saveBtn.disabled).toBe(false);
    saveBtn.click();

    await vi.waitFor(() => {
      expect(cashCountPosts.length).toBe(1);
    });
    expect(cashCountPosts[0]?.lines).toEqual([
      { currency: 'THB', denominationId: 11, qty: '0' },
    ]);
  });
});
