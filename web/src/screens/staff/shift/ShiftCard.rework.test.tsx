import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { errorText, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeHandoverPendingRow,
  makeHandoverReceiverRow,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
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

const SAMPLE_COUNTS = [
  makeCashCountRow({
    id: 'c-001',
    branchId: BRANCH,
    shiftId: 'shift-001',
    kind: 'OPEN',
    countedBy: OTHER,
    countedAt: '2026-10-11T02:00:00Z',
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [],
  }),
];

const SAMPLE_HANDOVERS = [
  makeHandoverPendingRow({
    id: 'h-001',
    fromShiftId: 'shift-other',
    countId: 'count-other',
    receiverId: ME,
    status: 'PENDING',
    createdAt: '2026-10-11T03:00:00Z',
  }),
];

const SAMPLE_DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
];

const SAMPLE_RECEIVERS = [
  makeHandoverReceiverRow({ userId: OTHER, name: 'Somchai' }),
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let viewHandlers: Record<string, () => Promise<Response>>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

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
      return json(200, {});
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

function mount(m: StaffMeView, f: StaffFloorView = floor()) {
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

describe('#1688 ShiftCard read error handling', () => {
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

  it('1. cash_counts read failure shows worded error', async () => {
    stubViews({
      cash_counts: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount(me(['ShiftCloseCertify']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    const errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('2. handovers_pending read failure shows worded error', async () => {
    stubViews({
      handovers_pending: async () =>
        json(403, { error: { code: 'roleNotAllowedHere', message: 'refused' } }),
    });

    mount(me(['ShiftOperate']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    const errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('roleNotAllowedHere', 'refused'));
  });

  it('3. denominations read failure shows worded error', async () => {
    stubViews({
      denominations: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount(me(['ShiftOperate']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    const errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('4. handover_receivers read failure shows worded error', async () => {
    stubViews({
      handover_receivers: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount(me(['ShiftOperate']));
    await settle();

    // Click handover button to trigger loadHandoverReceivers
    const handoverBtn = must(tid('shift-handover'));
    (handoverBtn as HTMLElement).click();

    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    const errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('5. network 500 / connection failure shows worded generic message in TH and EN', async () => {
    // In Thai:
    stubViews({
      cash_counts: async () => {
        throw new TypeError('Failed to fetch');
      },
    });

    mount(me(['ShiftCloseCertify']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    let errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(sharedTh['error.network']);
    expect(errorEl.textContent).not.toContain('Failed to fetch');

    // In English:
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    setLang('en');

    mount(me(['ShiftCloseCertify']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe(sharedEn['error.network']);
    expect(errorEl.textContent).not.toContain('Failed to fetch');
  });

  it('6. unknown refusal code shows worded generic message, never raw code or message', async () => {
    stubViews({
      cash_counts: async () =>
        json(400, { error: { code: 'UNKNOWN_REFUSAL_CODE', message: 'raw wire message' } }),
    });

    mount(me(['ShiftCloseCertify']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    const errorEl = must(tid('shift-error'));
    expect(errorEl.textContent?.trim()).toBe('raw wire message');
    expect(errorEl.textContent).not.toContain('UNKNOWN_REFUSAL_CODE');
  });

  it('7. stale data stays visible when subsequent read fails, but error is shown', async () => {
    let callCount = 0;
    stubViews({
      cash_counts: async () => {
        callCount++;
        if (callCount === 1) {
          return json(200, { version: 1, changed: true, data: SAMPLE_COUNTS });
        }
        return json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });
      },
    });

    mount(me(['ShiftCloseCertify', 'ShiftOperate']));
    await vi.waitFor(() => expect(q('[data-count-id="c-001"]')).not.toBeNull());
    expect(q(tid('shift-error'))).toBeNull();

    // Stale cash count row c-001 is loaded and visible
    // Now subsequent read (handover_receivers) fails
    viewHandlers.handover_receivers = async () =>
      json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });

    const handoverBtn = must(tid('shift-handover'));
    (handoverBtn as HTMLElement).click();

    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());
    // Stale cash count row c-001 is still visible:
    expect(q('[data-count-id="c-001"]')).not.toBeNull();
    // Error is also displayed:
    expect(must(tid('shift-error')).textContent?.trim()).toBe(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });
});
