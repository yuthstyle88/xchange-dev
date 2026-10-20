import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import {
  makeStaffCustomerRow,
  makeStaffFloorView,
  makeStaffHoldRow,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

const BRANCH = 'MAIN';

function me(permissions = ['CustomerHoldPlace', 'VerificationReview']) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function makeCtx(perms?: string[]): StaffCtx {
  return {
    me: me(perms),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const SAMPLE_HOLDS = [
  makeStaffHoldRow({
    id: 'hold-123',
    customerId: 'cust-456',
    customerName: 'Somchai Prasert',
    reason: 'Suspicious transaction pattern',
    status: 'OPEN',
    at: '2026-10-15T00:00:00Z',
  }),
];

const SAMPLE_CUSTOMERS = [
  makeStaffCustomerRow({
    id: 'cust-456',
    displayName: 'Somchai Prasert',
  }),
];

describe('HoldsCard: wordedError on failed calls (WE-41 #1790)', () => {
  let host: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host?.remove();
    host = undefined;
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 10));
    }
  }

  // ── Catch 1: loadHolds (mount) ─────────────────────────────────────────────
  describe('Catch 1: loadHolds', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      host!.innerHTML = '';
      dispose?.();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const errElEn = host!.querySelector('[data-testid="holds-error"]');
      expect(errElEn, 'holds-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal(
        'fetch',
        vi.fn().mockResolvedValue(
          json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'holds internal 500 failure',
            },
          }),
        ),
      );

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถอ่านรายการพักธุรกรรมได้';
      vi.stubGlobal(
        'fetch',
        vi.fn().mockResolvedValue(
          json(400, {
            error: {
              code: 'unkeyedLoadHoldsFailure',
              message: serverSentence,
            },
          }),
        ),
      );

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 2: handleFind (customer search) ──────────────────────────────────
  describe('Catch 2: handleFind', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));

      const findBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
      findBtn.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      findBtn.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="holds-error"]');
      expect(errElEn, 'holds-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'customer search 500 failure',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));

      const findBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
      findBtn.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ค้นหาลูกค้าไม่สำเร็จเนื่องจากระบบขัดข้อง';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return json(400, {
            error: {
              code: 'unkeyedFindFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));

      const findBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
      findBtn.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 3: handlePlace ───────────────────────────────────────────────────
  describe('Catch 3: handlePlace', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_CUSTOMERS });
        }
        if (url.includes('/api/customer-holds')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      // Search and pick customer
      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!.click();
      await flush();

      const custEl = host!.querySelector<HTMLDivElement>('[data-customer-id="cust-456"]')!;
      custEl.click();
      await flush();

      // Set reason and place
      const reasonInput = host!.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
      reasonInput.value = 'Hold test reason';
      reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

      const placeBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
      placeBtn.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      placeBtn.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="holds-error"]');
      expect(errElEn, 'holds-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_CUSTOMERS });
        }
        if (url.includes('/api/customer-holds')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'place hold 500 error',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!.click();
      await flush();

      host!.querySelector<HTMLDivElement>('[data-customer-id="cust-456"]')!.click();
      await flush();

      host!.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถบันทึกการพักธุรกรรมของลูกค้ารายนี้ได้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: [] });
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_CUSTOMERS });
        }
        if (url.includes('/api/customer-holds')) {
          return json(400, {
            error: {
              code: 'unkeyedPlaceFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const input = host!.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
      input.value = 'Somchai';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      host!.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!.click();
      await flush();

      host!.querySelector<HTMLDivElement>('[data-customer-id="cust-456"]')!.click();
      await flush();

      host!.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 4: handleReleaseConfirm ──────────────────────────────────────────
  describe('Catch 4: handleReleaseConfirm', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_HOLDS });
        }
        if (url.includes('/release')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const releaseBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-release"]')!;
      releaseBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      releaseBtn.click();
      await flush();
      const dialogEn = host!.querySelector('dialog')!;
      const buttonsEn = dialogEn.querySelectorAll('button');
      buttonsEn[1]!.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="holds-error"]');
      expect(errElEn, 'holds-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_HOLDS });
        }
        if (url.includes('/release')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'release 500 error',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const releaseBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-release"]')!;
      releaseBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถปลดการพักธุรกรรมได้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: false, data: SAMPLE_HOLDS });
        }
        if (url.includes('/release')) {
          return json(400, {
            error: {
              code: 'unkeyedReleaseFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <HoldsCard ctx={ctx} />, host!);
      await flush();

      const releaseBtn = host!.querySelector<HTMLButtonElement>('[data-testid="holds-release"]')!;
      releaseBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="holds-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });
});
