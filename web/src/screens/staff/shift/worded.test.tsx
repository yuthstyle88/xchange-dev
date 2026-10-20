import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { ShiftCard } from './ShiftCard';
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

function mockMe(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function mockFloor(over: Partial<StaffFloorView> = {}) {
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
let customHandler: ((url: string, method: string, init?: RequestInit) => Promise<Response> | Response) | null = null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function defaultFetch(url: string, method: string): Response {
  const path = url.split('?')[0]!;
  if (method === 'GET' && path.startsWith('/api/v3/views/')) {
    const viewName = path.slice('/api/v3/views/'.length);
    if (viewName === 'cash_counts') {
      return json(200, { version: 1, changed: true, data: SAMPLE_COUNTS });
    }
    if (viewName === 'handovers_pending') {
      return json(200, { version: 1, changed: true, data: SAMPLE_HANDOVERS });
    }
    if (viewName === 'denominations') {
      return json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });
    }
    if (viewName === 'handover_receivers') {
      return json(200, { version: 1, changed: true, data: { rows: SAMPLE_RECEIVERS } });
    }
    return json(404, { error: { code: 'notFound', message: 'not found' } });
  }
  return json(200, { id: 'ok-id' });
}

function installFetch() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (customHandler) {
        const customRes = await customHandler(url, method, init);
        if (customRes) return customRes;
      }
      return defaultFetch(url, method);
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

function mount(m: StaffMeView, f: StaffFloorView = mockFloor()) {
  const ctx: StaffCtx = { me: m, floor: f, refresh: vi.fn(async () => {}) };
  dispose = render(() => <ShiftCard ctx={ctx} />, host);
  return { ctx };
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

const UNKEYED_MESSAGE = 'ข้อผิดพลาดเฉพาะที่ไม่มีคีย์ร่วมจากเซิร์ฟเวอร์';

describe('WE-43: screens/staff/shift/ worded errors (web-worded-error.md §4.2)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    customHandler = null;
    installFetch();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
    customHandler = null;
  });

  describe('catch 1: loadCashCounts', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_counts')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      // EN
      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_counts')) {
          return json(500, { error: { code: 'serverError', message: 'Internal Server Error' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_counts')) {
          return json(400, { error: { code: 'unkeyedCashCountCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 2: loadPendingHandovers', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handovers_pending')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handovers_pending')) {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handovers_pending')) {
          return json(400, { error: { code: 'unkeyedHandoverCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 3: loadDenominations', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) {
          return json(400, { error: { code: 'unkeyedDenomsCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 4: loadHandoverReceivers', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handover_receivers')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handover_receivers')) {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/handover_receivers')) {
          return json(400, { error: { code: 'unkeyedReceiverCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 5: handleCountSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts') && method === 'POST') throw new TypeError('Failed to fetch');
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-close-count"]').click();
      await settle();
      must('[data-testid="shift-count-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-close-count"]').click();
      await settle();
      must('[data-testid="shift-count-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts') && method === 'POST') {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-close-count"]').click();
      await settle();
      must('[data-testid="shift-count-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedCountSubmitCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-close-count"]').click();
      await settle();
      must('[data-testid="shift-count-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 6: handleHandoverSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers') && method === 'POST') throw new TypeError('Failed to fetch');
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="shift-receiver"]');
      sel.value = OTHER;
      sel.dispatchEvent(new Event('input'));
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="shift-handover-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await settle();
      const selEn = must<HTMLSelectElement>('[data-testid="shift-receiver"]');
      selEn.value = OTHER;
      selEn.dispatchEvent(new Event('input'));
      selEn.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="shift-handover-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers') && method === 'POST') {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="shift-receiver"]');
      sel.value = OTHER;
      sel.dispatchEvent(new Event('input'));
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="shift-handover-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedHandoverSubmitCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await settle();
      must('[data-testid="shift-handover"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="shift-receiver"]');
      sel.value = OTHER;
      sel.dispatchEvent(new Event('input'));
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="shift-handover-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 7: handleVerifyCount', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts/c-001/verify') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-verify"]')).not.toBeNull());
      must('[data-testid="shift-verify"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-verify"]')).not.toBeNull());
      must('[data-testid="shift-verify"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts/c-001/verify') && method === 'POST') {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-verify"]')).not.toBeNull());
      must('[data-testid="shift-verify"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-counts/c-001/verify') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedVerifyCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftCloseCertify']));
      await vi.waitFor(() => expect(q('[data-testid="shift-verify"]')).not.toBeNull());
      must('[data-testid="shift-verify"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 8: handleAcceptHandover', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/accept') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-accept"]')).not.toBeNull());
      must('[data-testid="shift-accept"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-accept"]')).not.toBeNull());
      must('[data-testid="shift-accept"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/accept') && method === 'POST') {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-accept"]')).not.toBeNull());
      must('[data-testid="shift-accept"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/accept') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedAcceptCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-accept"]')).not.toBeNull());
      must('[data-testid="shift-accept"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 9: handleRejectHandover', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/reject') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-reject"]')).not.toBeNull());
      must('[data-testid="shift-reject"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-reject"]')).not.toBeNull());
      must('[data-testid="shift-reject"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/reject') && method === 'POST') {
          return json(500, { error: { code: 'serverError', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-reject"]')).not.toBeNull());
      must('[data-testid="shift-reject"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/handovers/h-001/reject') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedRejectCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount(mockMe(['ShiftOperate']));
      await vi.waitFor(() => expect(q('[data-testid="shift-reject"]')).not.toBeNull());
      must('[data-testid="shift-reject"]').click();
      await vi.waitFor(() => expect(q('[data-testid="shift-error"]')).not.toBeNull());
      expect(must('[data-testid="shift-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });
});
