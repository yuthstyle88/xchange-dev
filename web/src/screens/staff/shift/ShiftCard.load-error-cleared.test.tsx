import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { errorText, setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeHandoverPendingRow,
  makeHandoverReceiverRow,
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
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  });
}

const MY_SHIFT = {
  id: 'shift-open-001',
  openedAt: '2026-10-11T01:05:00Z',
};

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
let postHandlers: Record<string, (body: unknown) => Promise<Response>>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubApi(
  views: Record<string, () => Promise<Response>> = {},
  posts: Record<string, (body: unknown) => Promise<Response>> = {},
) {
  viewHandlers = {
    cash_counts: async () => json(200, { version: 1, changed: true, data: SAMPLE_COUNTS }),
    handovers_pending: async () => json(200, { version: 1, changed: true, data: SAMPLE_HANDOVERS }),
    denominations: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
    handover_receivers: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } }),
    ...views,
  };
  postHandlers = {
    ...posts,
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
      if (method === 'POST') {
        const body = init?.body ? JSON.parse(String(init.body)) : {};
        if (postHandlers[path]) {
          return postHandlers[path](body);
        }
        return json(200, { id: 'ok-001' });
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

describe('Staff Shift card: a background read\'s error banner is cleared upon success (#2741)', () => {
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

  it('Case 1: Denominations read fails on mount, press Open count, the re-read succeeds -> shift-error is gone; the count rows show', async () => {
    stubApi({
      denominations: async () =>
        json(500, { error: { code: 'serverError', message: 'Read failed' } }),
    });

    mount(me(['ShiftOperate']), floor({ myShift: null }));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());
    expect(q('[data-denomination-row]')).toBeNull();

    // Now denominations endpoint will succeed
    viewHandlers['denominations'] = async () =>
      json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

    // Click Open count button
    const openCountBtn = must(tid('shift-open-count'));
    (openCountBtn as HTMLElement).click();

    // Re-read succeeds: shift-error must be gone, count rows show
    await vi.waitFor(() => expect(q(tid('shift-error'))).toBeNull());
    expect(q('[data-denomination-row]')).not.toBeNull();
  });

  it('Case 2: Denominations read fails on mount, press Handover, the re-read succeeds -> shift-error is gone; the rows show', async () => {
    stubApi({
      denominations: async () =>
        json(500, { error: { code: 'serverError', message: 'Read failed' } }),
    });

    mount(me(['ShiftOperate']), floor({ myShift: MY_SHIFT }));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());
    expect(q('[data-denomination-row]')).toBeNull();

    // Now denominations endpoint will succeed
    viewHandlers['denominations'] = async () =>
      json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

    // Click Handover button
    const handoverBtn = must(tid('shift-handover'));
    (handoverBtn as HTMLElement).click();

    // Re-read succeeds: shift-error must be gone, rows show
    await vi.waitFor(() => expect(q(tid('shift-error'))).toBeNull());
    expect(q('[data-denomination-row]')).not.toBeNull();
  });

  it('Case 3: The re-read fails again -> The error stays', async () => {
    stubApi({
      denominations: async () =>
        json(500, { error: { code: 'serverError', message: 'Still failing' } }),
    });

    mount(me(['ShiftOperate']), floor({ myShift: null }));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    // Click Open count button; re-read still fails with 500
    const openCountBtn = must(tid('shift-open-count'));
    (openCountBtn as HTMLElement).click();
    await settle();

    // Error banner stays
    expect(q(tid('shift-error'))).not.toBeNull();
    expect(q(tid('shift-error'))!.textContent?.trim()).toBe(
      errorText('serverError', 'Still failing'),
    );
  });

  it('Case 4: The cash-counts read fails while the denominations read succeeds (mount) -> The cash-counts error stays', async () => {
    stubApi({
      cash_counts: async () =>
        json(500, { error: { code: 'serverError', message: 'Cash counts failed' } }),
      denominations: async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
    });

    // User has both ShiftCloseCertify (which loads cash_counts) and ShiftOperate (which loads denominations)
    mount(me(['ShiftCloseCertify', 'ShiftOperate']));
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());

    await settle();

    // Cash counts error stays despite denominations succeeding
    expect(q(tid('shift-error'))).not.toBeNull();
    expect(q(tid('shift-error'))!.textContent?.trim()).toBe(
      errorText('serverError', 'Cash counts failed'),
    );
  });

  it('Case 5: A write fails, then a loader succeeds -> The write\'s error stays', async () => {
    stubApi(
      {
        denominations: async () =>
          json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
        handover_receivers: async () =>
          json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } }),
      },
      {
        '/api/cash-counts': async () =>
          json(500, { error: { code: 'serverError', message: 'Write failed' } }),
      },
    );

    mount(me(['ShiftOperate']), floor({ myShift: MY_SHIFT }));
    await settle();

    // Close count and submit to trigger a failing write
    must(tid('shift-close-count')).click();
    await settle();

    must(tid('shift-count-save')).click();
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());
    expect(q(tid('shift-error'))!.textContent?.trim()).toBe(
      errorText('serverError', 'Write failed'),
    );

    // Now trigger a loader by clicking Handover (which loads handover_receivers)
    must(tid('shift-handover')).click();
    await settle();

    // The write's error stays!
    expect(q(tid('shift-error'))).not.toBeNull();
    expect(q(tid('shift-error'))!.textContent?.trim()).toBe(
      errorText('serverError', 'Write failed'),
    );
  });

  it('Receivers re-read: the receivers read fails on the first Handover press, then Handover is pressed again and the read succeeds -> shift-error is gone', async () => {
    stubApi({
      handover_receivers: async () =>
        json(500, { error: { code: 'serverError', message: 'Receivers read failed' } }),
    });

    mount(me(['ShiftOperate']), floor({ myShift: MY_SHIFT }));
    await settle();

    // Click Handover button; receivers read fails
    must(tid('shift-handover')).click();
    await vi.waitFor(() => expect(q(tid('shift-error'))).not.toBeNull());
    expect(q(tid('shift-error'))!.textContent?.trim()).toBe(
      errorText('serverError', 'Receivers read failed'),
    );

    // Now handover_receivers succeeds
    viewHandlers['handover_receivers'] = async () =>
      json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } });

    // Press Handover again to re-read receivers
    must(tid('shift-handover')).click();

    // Re-read succeeds: shift-error must be gone
    await vi.waitFor(() => expect(q(tid('shift-error'))).toBeNull());
  });

  describe('Write failure resets errorFrom: error banner stays after a later successful loader re-read', () => {
    it('1. count-submit failure preserves error banner on subsequent loader success', async () => {
      stubApi(
        {
          denominations: async () =>
            json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
          handover_receivers: async () =>
            json(400, { error: { code: 'receiversReadErr', message: 'Receivers read failed' } }),
        },
        {
          '/api/cash-counts': async () =>
            json(400, { error: { code: 'countSubmitErr', message: 'Count submit failed' } }),
        },
      );

      mount(me(['ShiftOperate']), floor({ myShift: MY_SHIFT }));
      await settle();

      // Handover read fails, setting errorFrom = 'loadHandoverReceivers'
      must(tid('shift-handover')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Receivers read failed'),
      );

      // Open Close count form and submit; count-submit fails
      must(tid('shift-close-count')).click();
      await settle();
      must(tid('shift-count-save')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Count submit failed'),
      );

      // Now handover_receivers endpoint succeeds
      viewHandlers['handover_receivers'] = async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } });

      // Click Handover again to trigger loadHandoverReceivers successfully
      must(tid('shift-handover')).click();
      await settle();

      // The write's error MUST stay
      expect(q(tid('shift-error'))).not.toBeNull();
      expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Count submit failed');
    });

    it('2. handover-start failure preserves error banner on subsequent loader success', async () => {
      stubApi(
        {
          denominations: async () =>
            json(400, { error: { code: 'denomsReadErr', message: 'Denoms read failed' } }),
          handover_receivers: async () =>
            json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } }),
        },
        {
          '/api/handovers': async () =>
            json(400, { error: { code: 'handoverStartErr', message: 'Handover start failed' } }),
        },
      );

      mount(me(['ShiftOperate']), floor({ myShift: MY_SHIFT }));
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Denoms read failed'),
      );

      // Open Handover form
      must(tid('shift-handover')).click();
      await settle();

      // Select receiver
      const select = must<HTMLSelectElement>(tid('shift-receiver'));
      select.value = OTHER;
      select.dispatchEvent(new Event('input', { bubbles: true }));
      select.dispatchEvent(new Event('change', { bubbles: true }));
      await settle();

      // Submit handover-start; write fails
      must(tid('shift-handover-save')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Handover start failed'),
      );

      // Now denominations endpoint will succeed
      viewHandlers['denominations'] = async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

      // Click Close count button to trigger loadDenominations re-read
      must(tid('shift-close-count')).click();
      await settle();

      // The write's error MUST stay
      expect(q(tid('shift-error'))).not.toBeNull();
      expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Handover start failed');
    });

    it('3. count-verify failure preserves error banner on subsequent loader success', async () => {
      stubApi(
        {
          denominations: async () =>
            json(400, { error: { code: 'denomsReadErr', message: 'Denoms read failed' } }),
          cash_counts: async () =>
            json(200, { version: 1, changed: true, data: SAMPLE_COUNTS }),
        },
        {
          '/api/cash-counts/c-001/verify': async () =>
            json(400, { error: { code: 'verifyErr', message: 'Verify failed' } }),
        },
      );

      mount(me(['ShiftCloseCertify', 'ShiftOperate']), floor({ myShift: null }));
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Denoms read failed'),
      );

      // Click verify button; write fails
      must(tid('shift-verify')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Verify failed'),
      );

      // Now denominations endpoint will succeed
      viewHandlers['denominations'] = async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

      // Click Open count button to trigger loadDenominations re-read
      must(tid('shift-open-count')).click();
      await settle();

      // The write's error MUST stay
      expect(q(tid('shift-error'))).not.toBeNull();
      expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Verify failed');
    });

    it('4. handover-accept failure preserves error banner on subsequent loader success', async () => {
      stubApi(
        {
          denominations: async () =>
            json(400, { error: { code: 'denomsReadErr', message: 'Denoms read failed' } }),
          handovers_pending: async () =>
            json(200, { version: 1, changed: true, data: SAMPLE_HANDOVERS }),
        },
        {
          '/api/handovers/h-001/accept': async () =>
            json(400, { error: { code: 'acceptErr', message: 'Accept failed' } }),
        },
      );

      mount(me(['ShiftOperate']), floor({ myShift: null }));
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Denoms read failed'),
      );

      // Click accept button; write fails
      must(tid('shift-accept')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Accept failed'),
      );

      // Now denominations endpoint will succeed
      viewHandlers['denominations'] = async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

      // Click Open count button to trigger loadDenominations re-read
      must(tid('shift-open-count')).click();
      await settle();

      // The write's error MUST stay
      expect(q(tid('shift-error'))).not.toBeNull();
      expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Accept failed');
    });

    it('5. handover-reject failure preserves error banner on subsequent loader success', async () => {
      stubApi(
        {
          denominations: async () =>
            json(400, { error: { code: 'denomsReadErr', message: 'Denoms read failed' } }),
          handovers_pending: async () =>
            json(200, { version: 1, changed: true, data: SAMPLE_HANDOVERS }),
        },
        {
          '/api/handovers/h-001/reject': async () =>
            json(400, { error: { code: 'rejectErr', message: 'Reject failed' } }),
        },
      );

      mount(me(['ShiftOperate']), floor({ myShift: null }));
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Denoms read failed'),
      );

      // Click reject button; write fails
      must(tid('shift-reject')).click();
      await vi.waitFor(() =>
        expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Reject failed'),
      );

      // Now denominations endpoint will succeed
      viewHandlers['denominations'] = async () =>
        json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });

      // Click Open count button to trigger loadDenominations re-read
      must(tid('shift-open-count')).click();
      await settle();

      // The write's error MUST stay
      expect(q(tid('shift-error'))).not.toBeNull();
      expect(q(tid('shift-error'))!.textContent?.trim()).toBe('Reject failed');
    });
  });
});
