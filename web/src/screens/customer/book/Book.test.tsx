// Written before the implementation by the test writer gemini/warut-3 (ticket author claude/e92db6), against BK-6 provided tests 7f8e56d1. Do not edit, delete or weaken.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
}

let fs: NodeFs;
let path: NodePath;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  const pathName = 'node:path';
  const urlName = 'node:url';
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const { fileURLToPath } = (await import(/* @vite-ignore */ urlName)) as unknown as {
    fileURLToPath: (u: string) => string;
  };
  dir = path.dirname(fileURLToPath(import.meta.url)) + '/';
});

// ── BK-6 Test Shapes & JSON Literals (No Generated Types) ─────────────────────

const SAMPLE_BRANCHES = {
  branches: [
    {
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน 991 ถ.พระราม 1 แขวงปทุมวัน เขตปทุมวัน กรุงเทพฯ 10330',
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      id: 'XC-B02',
      code: 'XC-B02',
      name: 'สาขาสนามบินดอนเมือง',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

const SAMPLE_SLOTS = {
  version: 1,
  changed: true,
  data: {
    slots: [
      {
        id: '00000000-0000-4000-8000-000000000001',
        start: '2026-10-15T03:00:00+00:00',
        end: '2026-10-15T03:30:00+00:00',
        available: 3,
      },
      {
        id: '00000000-0000-4000-8000-000000000002',
        start: '2026-10-15T03:30:00+00:00',
        end: '2026-10-15T04:00:00+00:00',
        available: 0,
      },
    ],
  },
};

const SAMPLE_PLANNER_PREVIEW = {
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-15',
};

const SAMPLE_BOOKING_CREATE_SUCCESS = {
  id: '11111111-1111-4000-8000-000000000001',
  status: 'CONFIRMED',
  sourceChannel: 'APP',
  branchId: 'XC-B01',
  slotId: '00000000-0000-4000-8000-000000000001',
  shopSide: 'BUY',
  currency: 'USD',
  quantity: 100.0,
  qrToken: 'qr-token-test-xyz',
};

interface RecordedCall {
  method: string;
  url: string;
  path: string;
  headers: Record<string, string>;
  body: any;
}

function setupMockBackend(options: {
  createResponse?: { status: number; body: any };
  slotsResponse?: { status: number; body: any };
  branchesResponse?: { status: number; body: any };
  previewResponse?: { status: number; body: any };
} = {}) {
  const calls: RecordedCall[] = [];

  const fetchHandler = vi.fn(async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const urlObj = new URL(rawUrl, 'http://xc.test');
    const path = urlObj.pathname;
    const method = (init?.method ?? 'GET').toUpperCase();

    const headers: Record<string, string> = {};
    if (init?.headers) {
      if (init.headers instanceof Headers) {
        init.headers.forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      } else if (Array.isArray(init.headers)) {
        init.headers.forEach(([k, v]) => {
          headers[k.toLowerCase()] = v;
        });
      } else {
        Object.entries(init.headers).forEach(([k, v]) => {
          headers[k.toLowerCase()] = String(v);
        });
      }
    }

    let parsedBody: any = null;
    if (init?.body && typeof init.body === 'string') {
      try {
        parsedBody = JSON.parse(init.body);
      } catch {
        parsedBody = init.body;
      }
    }

    calls.push({ method, url: rawUrl, path, headers, body: parsedBody });

    // Host endpoint
    if (path === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC', env: 'test' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // Public branches
    if (
      path === '/api/public/orgs/XC/branches' ||
      path.startsWith('/api/public/orgs/') ||
      path === '/api/v3/views/public_branches'
    ) {
      const resp = options.branchesResponse ?? { status: 200, body: SAMPLE_BRANCHES };
      return new Response(JSON.stringify(resp.body), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // Booking slots
    if (path === '/api/v3/views/booking_slots') {
      const resp = options.slotsResponse ?? { status: 200, body: SAMPLE_SLOTS };
      return new Response(JSON.stringify(resp.body), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // Planner preview
    if (path === '/api/v3/compute/planner_preview') {
      const resp = options.previewResponse ?? { status: 200, body: SAMPLE_PLANNER_PREVIEW };
      return new Response(JSON.stringify(resp.body), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // Booking create
    if (path === '/api/bookings' && method === 'POST') {
      const resp = options.createResponse ?? { status: 200, body: SAMPLE_BOOKING_CREATE_SUCCESS };
      return new Response(JSON.stringify(resp.body), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response(JSON.stringify({ error: { code: 'notFound', message: path } }), {
      status: 404,
      headers: { 'Content-Type': 'application/json' },
    });
  });

  vi.stubGlobal('fetch', fetchHandler);
  return { calls, fetchHandler };
}

// ── Element Selectors ─────────────────────────────────────────────────────────

function getReferralInput(container: HTMLElement): HTMLInputElement | null {
  return (
    container.querySelector<HTMLInputElement>('input[name="referralCode"]') ??
    container.querySelector<HTMLInputElement>('input[id="referralCode"]') ??
    container.querySelector<HTMLInputElement>('input[data-testid="referral-code"]') ??
    container.querySelector<HTMLInputElement>('input[placeholder*="referral" i]') ??
    container.querySelector<HTMLInputElement>('input[placeholder*="โค้ดแนะนำ" i]')
  );
}

function getSubmitButton(container: HTMLElement): HTMLButtonElement | null {
  return (
    container.querySelector<HTMLButtonElement>('button[type="submit"]') ??
    container.querySelector<HTMLButtonElement>('button[data-action="book"]') ??
    container.querySelector<HTMLButtonElement>('[data-testid="book-submit"]')
  );
}

function getBranchSelect(container: HTMLElement): HTMLSelectElement | null {
  return (
    container.querySelector<HTMLSelectElement>('select[name="branchId"]') ??
    container.querySelector<HTMLSelectElement>('select[name="branch"]') ??
    container.querySelector<HTMLSelectElement>('[data-testid="branch-select"]')
  );
}

function getQuantityInput(container: HTMLElement): HTMLInputElement | null {
  return (
    container.querySelector<HTMLInputElement>('input[name="quantity"]') ??
    container.querySelector<HTMLInputElement>('input[name="amount"]') ??
    container.querySelector<HTMLInputElement>('[data-testid="quantity-input"]')
  );
}

function getSlotOption(container: HTMLElement): HTMLElement | null {
  return (
    container.querySelector('[data-testid="slot-option"]') ??
    container.querySelector('input[name="slotId"]') ??
    container.querySelector('.slot-card') ??
    container.querySelector('.slot-option') ??
    container.querySelector('[data-slot-id]') ??
    container.querySelector('select[name="slotId"] option:not([value=""])')
  );
}

function getCurrencySelect(container: HTMLElement): HTMLSelectElement | HTMLInputElement | null {
  return (
    container.querySelector<HTMLSelectElement>('select[name="currency"]') ??
    container.querySelector<HTMLSelectElement>('[data-testid="currency-select"]') ??
    container.querySelector<HTMLInputElement>('input[name="currency"]')
  );
}

function getShopSideSelect(container: HTMLElement): HTMLElement | null {
  return (
    container.querySelector<HTMLSelectElement>('select[name="shopSide"]') ??
    container.querySelector<HTMLInputElement>('input[name="shopSide"][value="BUY"]') ??
    container.querySelector('[data-testid="side-buy"]')
  );
}

// Eager glob for optional i18n module
const i18nModules = import.meta.glob<Record<string, unknown>>('./i18n.ts', { eager: true });

describe('APP-16 customer C02 book screen (/app/book) up-front tests (#985)', () => {
  let root: HTMLDivElement;

  beforeEach(() => {
    setLang('th');
    setAudience('customer');
    vi.resetModules();
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
  });

  afterEach(() => {
    root.remove();
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function bootScreen(url = '/app/book') {
    window.history.replaceState({}, '', url);
    await import('./main');
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('referral field is optional: booking with empty referral code sends NO referralCode key in payload', async () => {
    const { calls } = setupMockBackend();
    await bootScreen('/app/book');

    const refInput = getReferralInput(root);
    expect(refInput, 'referral code input must exist in the booking screen').not.toBeNull();
    expect(refInput!.value, 'referral field starts empty without ?ref=').toBe('');

    // Require branch, slot, and quantity controls per review item 3
    const branchSel = getBranchSelect(root);
    expect(branchSel, 'branch selection control must exist in the booking screen').not.toBeNull();
    branchSel!.value = 'XC-B01';
    branchSel!.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const opt =
        root.querySelector<HTMLOptionElement>(
          `option[value="${SAMPLE_SLOTS.data.slots[0].id}"], input[value="${SAMPLE_SLOTS.data.slots[0].id}"]`,
        ) ?? getSlotOption(root);
      expect(opt, 'available slot selection control must exist in the booking screen').not.toBeNull();
      if (opt instanceof HTMLOptionElement) {
        expect(opt.value).toBe(SAMPLE_SLOTS.data.slots[0].id);
      }
    });

    const slotOpt = getSlotOption(root);
    expect(slotOpt, 'available slot selection control must exist in the booking screen').not.toBeNull();
    if (slotOpt instanceof HTMLOptionElement) {
      (slotOpt.parentElement as HTMLSelectElement).value = '00000000-0000-4000-8000-000000000001';
      slotOpt.parentElement!.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (slotOpt instanceof HTMLInputElement) {
      slotOpt.checked = true;
      slotOpt.dispatchEvent(new Event('change', { bubbles: true }));
      slotOpt.click();
    } else {
      slotOpt!.click();
    }

    const curSel = getCurrencySelect(root);
    if (curSel instanceof HTMLSelectElement) {
      curSel.value = 'USD';
      curSel.dispatchEvent(new Event('change', { bubbles: true }));
    }
    const sideSel = getShopSideSelect(root);
    if (sideSel instanceof HTMLSelectElement) {
      sideSel.value = 'BUY';
      sideSel.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (sideSel instanceof HTMLElement) {
      sideSel.click();
    }

    const qtyInput = getQuantityInput(root);
    expect(qtyInput, 'quantity input control must exist in the booking screen').not.toBeNull();
    qtyInput!.value = '100';
    qtyInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = getSubmitButton(root);
    expect(submitBtn, 'booking submit button must exist in the booking screen').not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      const createCall = calls.find((c) => c.path === '/api/bookings' && c.method === 'POST');
      expect(createCall, 'POST /api/bookings must be called').toBeDefined();
    });

    const createCall = calls.find((c) => c.path === '/api/bookings' && c.method === 'POST')!;

    // Contract: BK-6 create body is {branchId, slotId, shopSide, currency, quantity}
    expect(createCall.body.branchId, 'create body must contain branchId').toBe('XC-B01');
    expect(createCall.body.branch, '#1913: every booking command names its branch').toBe('XC-B01');
    expect(createCall.body.slotId, 'create body must contain slotId').toBe('00000000-0000-4000-8000-000000000001');
    expect(createCall.body.shopSide, 'create body must contain shopSide BUY').toBe('BUY');
    expect(createCall.body.currency, 'create body must contain currency USD').toBe('USD');
    expect(typeof createCall.body.quantity, 'quantity must be sent as a JSON number').toBe('number');
    expect(createCall.body.quantity, 'quantity must match typed value 100').toBe(100);

    // Contract: optional referral field - empty sends NO referralCode property
    expect(
      'referralCode' in createCall.body,
      'empty referral field must NOT send referralCode key in payload (design §9.1 APP-16)',
    ).toBe(false);

    // Contract: customer APP path sends NO quoteId (customer cannot make quotes, design P5, PR 984)
    expect(
      'quoteId' in createCall.body,
      'bookingCreate from customer app sends NO quoteId (design §4.5, PR 984)',
    ).toBe(false);

    // Contract: CommandAction carries Idempotency-Key
    expect(
      createCall.headers['idempotency-key'],
      'bookingCreate must include Idempotency-Key header',
    ).toBeDefined();
  });

  it('URL query parameter ?ref=AB12CD prefills the referral code input, while a plain visit starts empty', async () => {
    setupMockBackend();

    // 1. Visit with ?ref=AB12CD
    await bootScreen('/app/book?ref=AB12CD');

    const refInputPrefilled = getReferralInput(root);
    expect(refInputPrefilled, 'referral code input must exist when ?ref= is in URL').not.toBeNull();
    expect(
      refInputPrefilled!.value,
      '?ref=AB12CD must prefill the referral code input (design §4.5, §9.1)',
    ).toBe('AB12CD');

    // 2. Fresh visit without ?ref=
    root.remove();
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
    vi.resetModules();

    await bootScreen('/app/book');

    const refInputEmpty = getReferralInput(root);
    expect(refInputEmpty, 'referral code input must exist on plain visit').not.toBeNull();
    expect(
      refInputEmpty!.value,
      'referral code field must start empty without ?ref= in URL',
    ).toBe('');
  });

  it('referral code value is sent exactly as typed without client-side case conversion or trimming (server normalises)', async () => {
    const { calls } = setupMockBackend();
    // Prefill with mixed-case and lowercase
    await bootScreen('/app/book?ref=ab12cd_xyz');

    const branchSel = getBranchSelect(root);
    expect(branchSel, 'branch selection control must exist in the booking screen').not.toBeNull();
    branchSel!.value = 'XC-B01';
    branchSel!.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const opt =
        root.querySelector<HTMLOptionElement>(
          `option[value="${SAMPLE_SLOTS.data.slots[0].id}"], input[value="${SAMPLE_SLOTS.data.slots[0].id}"]`,
        ) ?? getSlotOption(root);
      expect(opt, 'available slot selection control must exist in the booking screen').not.toBeNull();
      if (opt instanceof HTMLOptionElement) {
        expect(opt.value).toBe(SAMPLE_SLOTS.data.slots[0].id);
      }
    });

    const slotOpt = getSlotOption(root);
    expect(slotOpt, 'available slot selection control must exist in the booking screen').not.toBeNull();
    if (slotOpt instanceof HTMLOptionElement) {
      (slotOpt.parentElement as HTMLSelectElement).value = '00000000-0000-4000-8000-000000000001';
      slotOpt.parentElement!.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (slotOpt instanceof HTMLInputElement) {
      slotOpt.checked = true;
      slotOpt.dispatchEvent(new Event('change', { bubbles: true }));
      slotOpt.click();
    } else {
      slotOpt!.click();
    }

    const qtyInput = getQuantityInput(root);
    expect(qtyInput, 'quantity input control must exist in the booking screen').not.toBeNull();
    qtyInput!.value = '100';
    qtyInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const refInput = getReferralInput(root);
    expect(refInput, 'referral input must exist').not.toBeNull();
    expect(refInput!.value).toBe('ab12cd_xyz');

    const submitBtn = getSubmitButton(root);
    expect(submitBtn, 'booking submit button must exist').not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      const createCall = calls.find((c) => c.path === '/api/bookings' && c.method === 'POST');
      expect(createCall, 'POST /api/bookings must be called').toBeDefined();
    });

    const createCall = calls.find((c) => c.path === '/api/bookings' && c.method === 'POST')!;
    // Contract: value sent as typed; server normalises via ReferralCode (design §4.5, §9.1, RA-1 #979)
    expect(
      createCall.body.referralCode,
      'referral code must be sent exactly as typed (no premature uppercase or trim)',
    ).toBe('ab12cd_xyz');
    expect(createCall.body.branchId).toBe('XC-B01');
    expect(createCall.body.branch, '#1913: every booking command names its branch').toBe('XC-B01');
    expect(createCall.body.slotId).toBe('00000000-0000-4000-8000-000000000001');
    expect(typeof createCall.body.quantity).toBe('number');
  });

  it('stores nothing for referral code: no localStorage, sessionStorage, or cookie storage API is used', async () => {
    const localGetSpy = vi.spyOn(Storage.prototype, 'getItem');
    const localSetSpy = vi.spyOn(Storage.prototype, 'setItem');
    const sessionGetSpy = vi.spyOn(window.sessionStorage, 'getItem');
    const sessionSetSpy = vi.spyOn(window.sessionStorage, 'setItem');

    setupMockBackend();
    await bootScreen('/app/book?ref=AB12CD');

    const refInput = getReferralInput(root);
    expect(refInput, 'referral input must exist').not.toBeNull();

    // Check no storage API was called for the referral code
    const isReferralKey = (key: string | null) =>
      typeof key === 'string' &&
      (key.toLowerCase().includes('ref') || key.toLowerCase().includes('referral') || key.includes('AB12CD'));

    const localSetWithRef = localSetSpy.mock.calls.some(([k]) => isReferralKey(k));
    const localGetWithRef = localGetSpy.mock.calls.some(([k]) => isReferralKey(k));
    const sessionSetWithRef = sessionSetSpy.mock.calls.some(([k]) => isReferralKey(k));
    const sessionGetWithRef = sessionGetSpy.mock.calls.some(([k]) => isReferralKey(k));

    expect(localSetWithRef, 'localStorage.setItem must not be called for referral code (design §4.5)').toBe(false);
    expect(localGetWithRef, 'localStorage.getItem must not be called for referral code (design §4.5)').toBe(false);
    expect(sessionSetWithRef, 'sessionStorage.setItem must not be called for referral code (design §4.5)').toBe(false);
    expect(sessionGetWithRef, 'sessionStorage.getItem must not be called for referral code (design §4.5)').toBe(false);

    expect(document.cookie, 'document.cookie must not store referral code').not.toContain('AB12CD');
  });

  it('validationFailed on referralCode is shown beside the field and keeps the form fields populated', async () => {
    // BK-6 / Referral validation error shape
    const validationErrorBody = {
      error: {
        code: 'validationFailed',
        message: 'รหัสแนะนำไม่ถูกต้อง',
        field: 'referralCode',
      },
    };

    setupMockBackend({
      createResponse: { status: 400, body: validationErrorBody },
    });

    await bootScreen('/app/book?ref=INVALID_CODE');

    const branchSel = getBranchSelect(root);
    expect(branchSel, 'branch selection control must exist in the booking screen').not.toBeNull();
    branchSel!.value = 'XC-B01';
    branchSel!.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const opt =
        root.querySelector<HTMLOptionElement>(
          `option[value="${SAMPLE_SLOTS.data.slots[0].id}"], input[value="${SAMPLE_SLOTS.data.slots[0].id}"]`,
        ) ?? getSlotOption(root);
      expect(opt, 'available slot selection control must exist in the booking screen').not.toBeNull();
      if (opt instanceof HTMLOptionElement) {
        expect(opt.value).toBe(SAMPLE_SLOTS.data.slots[0].id);
      }
    });

    const slotOpt = getSlotOption(root);
    expect(slotOpt, 'available slot selection control must exist in the booking screen').not.toBeNull();
    if (slotOpt instanceof HTMLOptionElement) {
      (slotOpt.parentElement as HTMLSelectElement).value = '00000000-0000-4000-8000-000000000001';
      slotOpt.parentElement!.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (slotOpt instanceof HTMLInputElement) {
      slotOpt.checked = true;
      slotOpt.dispatchEvent(new Event('change', { bubbles: true }));
      slotOpt.click();
    } else {
      slotOpt!.click();
    }

    const refInput = getReferralInput(root);
    expect(refInput, 'referral input must exist').not.toBeNull();
    expect(refInput!.value).toBe('INVALID_CODE');

    const qtyInput = getQuantityInput(root);
    expect(qtyInput, 'quantity input control must exist in the booking screen').not.toBeNull();
    qtyInput!.value = '250';
    qtyInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = getSubmitButton(root);
    expect(submitBtn, 'booking submit button must exist').not.toBeNull();
    submitBtn!.click();

    // Await error indication per review item 1
    await vi.waitFor(() => {
      const fieldWrapper = refInput!.closest('[data-field="referralCode"], .field, .form-group, label');
      const fieldErrorEl = fieldWrapper?.querySelector(
        '[role="alert"], [data-error], .error, .field-error, .text-error, .invalid-feedback',
      );
      const isAriaInvalid = refInput!.getAttribute('aria-invalid') === 'true';

      const i18n = i18nModules['./i18n.ts'] as { th?: Record<string, string> } | undefined;
      const expectedText =
        i18n?.th?.['error.invalidReferralCode'] ??
        i18n?.th?.['error.referralCode'] ??
        i18n?.th?.['error.validationFailed'] ??
        i18n?.th?.['invalidReferralCode'] ??
        'รหัสแนะนำไม่ถูกต้อง';

      const hasWordedText = fieldErrorEl != null && fieldErrorEl.textContent?.includes(expectedText);

      expect(
        isAriaInvalid || hasWordedText,
        'validationFailed on referralCode must set aria-invalid="true" on input or show worded error inside field wrapper (design §9.1 APP-16)',
      ).toBe(true);

      // Raw code validationFailed must NOT be displayed
      expect(root.textContent).not.toContain('validationFailed');
    });

    // Form values must be kept intact upon validation failure (not reset)
    expect(
      refInput!.value,
      'referral code input value must be retained after validation error',
    ).toBe('INVALID_CODE');

    expect(
      qtyInput!.value,
      'other form fields must remain populated after validation error',
    ).toBe('250');
  });

  it('sends Idempotency-Key header and reuses the exact same key across retries of the same booking', async () => {
    let callCount = 0;
    const recordedKeys: string[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = new URL(rawUrl, 'http://xc.test').pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/booking_slots') {
          return new Response(JSON.stringify(SAMPLE_SLOTS), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path === '/api/public/host') {
          return new Response(JSON.stringify({ org: 'XC' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path.startsWith('/api/public/orgs/') || path.includes('/views/')) {
          return new Response(JSON.stringify(SAMPLE_BRANCHES), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path === '/api/bookings' && method === 'POST') {
          callCount++;
          let key = '';
          if (init?.headers) {
            if (init.headers instanceof Headers) {
              key = init.headers.get('idempotency-key') ?? '';
            } else if (Array.isArray(init.headers)) {
              key = init.headers.find(([k]) => k.toLowerCase() === 'idempotency-key')?.[1] ?? '';
            } else {
              key = (init.headers as Record<string, string>)['idempotency-key'] ?? '';
            }
          }
          recordedKeys.push(key);

          if (callCount === 1) {
            // First call fails with transient network or server error
            return new Response(JSON.stringify({ error: { code: 'serverBusy', message: 'Busy' } }), {
              status: 503,
              headers: { 'Content-Type': 'application/json' },
            });
          }
          return new Response(JSON.stringify(SAMPLE_BOOKING_CREATE_SUCCESS), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        return new Response('Not Found', { status: 404 });
      }),
    );

    await bootScreen('/app/book');

    const branchSel = getBranchSelect(root);
    expect(branchSel, 'branch selection control must exist in the booking screen').not.toBeNull();
    branchSel!.value = 'XC-B01';
    branchSel!.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const opt =
        root.querySelector<HTMLOptionElement>(
          `option[value="${SAMPLE_SLOTS.data.slots[0].id}"], input[value="${SAMPLE_SLOTS.data.slots[0].id}"]`,
        ) ?? getSlotOption(root);
      expect(opt, 'available slot selection control must exist in the booking screen').not.toBeNull();
      if (opt instanceof HTMLOptionElement) {
        expect(opt.value).toBe(SAMPLE_SLOTS.data.slots[0].id);
      }
    });

    const slotOpt = getSlotOption(root);
    expect(slotOpt, 'available slot selection control must exist in the booking screen').not.toBeNull();
    if (slotOpt instanceof HTMLOptionElement) {
      (slotOpt.parentElement as HTMLSelectElement).value = '00000000-0000-4000-8000-000000000001';
      slotOpt.parentElement!.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (slotOpt instanceof HTMLInputElement) {
      slotOpt.checked = true;
      slotOpt.dispatchEvent(new Event('change', { bubbles: true }));
      slotOpt.click();
    } else {
      slotOpt!.click();
    }

    const qtyInput = getQuantityInput(root);
    expect(qtyInput, 'quantity input control must exist in the booking screen').not.toBeNull();
    qtyInput!.value = '100';
    qtyInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = getSubmitButton(root);
    expect(submitBtn, 'booking submit button must exist in the booking screen').not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(callCount, 'first booking create attempt sent').toBe(1);
    });

    const firstKey = recordedKeys[0];
    expect(firstKey, 'first attempt sends non-empty Idempotency-Key header').toBeTruthy();

    // User retries submitting without modifying form
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(callCount, 'retry booking create attempt sent').toBe(2);
    });

    const secondKey = recordedKeys[1];
    expect(
      secondKey,
      'retry attempt must reuse the exact same Idempotency-Key header across retries',
    ).toBe(firstKey);
  });

  it('displays worded server errors for domain refusals (slotUnavailable, capacityFull, bookingsClosed)', async () => {
    let currentRefusal = 'slotUnavailable';
    let currentMessage = 'ช่วงเวลาที่เลือกไม่พร้อมใช้งานแล้ว';

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = new URL(rawUrl, 'http://xc.test').pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/booking_slots') {
          return new Response(JSON.stringify(SAMPLE_SLOTS), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path === '/api/public/host') {
          return new Response(JSON.stringify({ org: 'XC' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path.startsWith('/api/public/orgs/') || path.includes('/views/')) {
          return new Response(JSON.stringify(SAMPLE_BRANCHES), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
        }

        if (path === '/api/bookings' && method === 'POST') {
          return new Response(
            JSON.stringify({ error: { code: currentRefusal, message: currentMessage } }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        return new Response('Not Found', { status: 404 });
      }),
    );

    await bootScreen('/app/book');

    const branchSel = getBranchSelect(root);
    expect(branchSel, 'branch selection control must exist in the booking screen').not.toBeNull();
    branchSel!.value = 'XC-B01';
    branchSel!.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const opt =
        root.querySelector<HTMLOptionElement>(
          `option[value="${SAMPLE_SLOTS.data.slots[0].id}"], input[value="${SAMPLE_SLOTS.data.slots[0].id}"]`,
        ) ?? getSlotOption(root);
      expect(opt, 'available slot selection control must exist in the booking screen').not.toBeNull();
      if (opt instanceof HTMLOptionElement) {
        expect(opt.value).toBe(SAMPLE_SLOTS.data.slots[0].id);
      }
    });

    const slotOpt = getSlotOption(root);
    expect(slotOpt, 'available slot selection control must exist in the booking screen').not.toBeNull();
    if (slotOpt instanceof HTMLOptionElement) {
      (slotOpt.parentElement as HTMLSelectElement).value = '00000000-0000-4000-8000-000000000001';
      slotOpt.parentElement!.dispatchEvent(new Event('change', { bubbles: true }));
    } else if (slotOpt instanceof HTMLInputElement) {
      slotOpt.checked = true;
      slotOpt.dispatchEvent(new Event('change', { bubbles: true }));
      slotOpt.click();
    } else {
      slotOpt!.click();
    }

    const qtyInput = getQuantityInput(root);
    expect(qtyInput, 'quantity input control must exist in the booking screen').not.toBeNull();
    qtyInput!.value = '100';
    qtyInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = getSubmitButton(root);
    expect(submitBtn, 'booking submit button must exist in the booking screen').not.toBeNull();

    // Domain refusal error codes tested per review item 2
    const refusals = [
      { code: 'slotUnavailable', msg: 'ช่วงเวลาที่เลือกไม่พร้อมใช้งานแล้ว' },
      { code: 'capacityFull', msg: 'ที่นั่งในช่วงเวลานี้เต็มแล้ว' },
      { code: 'bookingsClosed', msg: 'ระบบการจองปิดให้บริการในขณะนี้' },
    ];

    for (const refusal of refusals) {
      currentRefusal = refusal.code;
      currentMessage = refusal.msg;
      submitBtn!.click();

      await vi.waitFor(() => {
        const i18n = i18nModules['./i18n.ts'] as { th?: Record<string, string> } | undefined;
        const expectedText =
          i18n?.th?.[`error.${refusal.code}`] ??
          i18n?.th?.[`err.${refusal.code}`] ??
          i18n?.th?.[refusal.code];

        expect(
          expectedText,
          `i18n.ts th must define translation for error.${refusal.code}`,
        ).toBeDefined();

        expect(root.textContent).toContain(expectedText);
        expect(root.textContent).not.toContain(refusal.code); // raw error code must NOT be shown
      });
    }
  });

  it('defines localized translation keys across TH, EN, and ZH with identical key sets', () => {
    const mod = i18nModules['./i18n.ts'] as
      | {
          th?: Record<string, string>;
          en?: Record<string, string>;
          zh?: Record<string, string>;
        }
      | undefined;

    expect(mod, 'i18n.ts must be implemented and export th, en, zh').toBeDefined();
    expect(mod?.th, 'th translations must be defined').toBeDefined();
    expect(mod?.en, 'en translations must be defined').toBeDefined();
    expect(mod?.zh, 'zh translations must be defined').toBeDefined();

    const th = mod!.th!;
    const en = mod!.en!;
    const zh = mod!.zh!;

    const thKeys = Object.keys(th).sort();
    const enKeys = Object.keys(en).sort();
    const zhKeys = Object.keys(zh).sort();

    expect(enKeys, 'en must have identical keys as th').toEqual(thKeys);
    expect(zhKeys, 'zh must have identical keys as th').toEqual(thKeys);

    // Required domain keys for book screen
    expect(th, 'must contain referral code label/placeholder key').toHaveProperty('referralCode');

    // Refusal error keys (th['error.<code>'] per review feedback)
    const hasSlotUnavailable = 'error.slotUnavailable' in th || 'slotUnavailable' in th;
    const hasCapacityFull = 'error.capacityFull' in th || 'capacityFull' in th;
    const hasBookingsClosed = 'error.bookingsClosed' in th || 'bookingsClosed' in th;
    expect(hasSlotUnavailable, 'must contain error.slotUnavailable key').toBe(true);
    expect(hasCapacityFull, 'must contain error.capacityFull key').toBe(true);
    expect(hasBookingsClosed, 'must contain error.bookingsClosed key').toBe(true);
  });

  it('web/pages/app-book.html and main.tsx fulfill the customer shell and gate contract (design §4.5, LK-1)', () => {
    // 1. Entry HTML verification
    const htmlPath = dir + '../../../../pages/app-book.html';
    expect(fs.existsSync(htmlPath), 'web/pages/app-book.html must exist').toBe(true);
    const htmlContent = fs.readFileSync(htmlPath, 'utf-8');

    expect(htmlContent, 'declares xc-path as /app/book').toMatch(/<meta\s+name=["']xc-path["']\s+content=["']\/app\/book["']/);
    expect(htmlContent, 'declares xc-gate as anyone').toMatch(/<meta\s+name=["']xc-gate["']\s+content=["']anyone["']/);
    expect(htmlContent, 'references /src/screens/customer/book/main.tsx').toContain('/src/screens/customer/book/main.tsx');

    // 2. main.tsx source verification
    const mainPath = dir + 'main.tsx';
    expect(fs.existsSync(mainPath), 'web/src/screens/customer/book/main.tsx must exist').toBe(true);
    const mainSource = fs.readFileSync(mainPath, 'utf-8');

    // Calls setAudience('customer') before render (languages.md §3.1, audience.test.tsx)
    const audienceIdx = mainSource.indexOf("setAudience('customer')");
    const renderIdx = mainSource.indexOf('render(');
    expect(audienceIdx, "main.tsx must call setAudience('customer')").toBeGreaterThanOrEqual(0);
    expect(renderIdx, 'main.tsx must call render(').toBeGreaterThanOrEqual(0);
    expect(audienceIdx, "setAudience('customer') must precede render(").toBeLessThan(renderIdx);

    // Shell contract: imports CustomerShell from ../_shell/Shell and renders <CustomerShell route="/app/book">
    expect(
      mainSource,
      'main.tsx must import CustomerShell from ../_shell/Shell',
    ).toMatch(/import\s*\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s*['"][./]*_shell\/Shell['"]/);
    expect(
      mainSource,
      'main.tsx must render <CustomerShell route="/app/book">',
    ).toMatch(/<CustomerShell\s+route=["']\/app\/book["']/);

    // Must NOT import AppShell directly (LK-1 customer screen rule)
    expect(mainSource, 'main.tsx must not import AppShell directly').not.toMatch(
      /import\s*\{[^}]*\bAppShell\b[^}]*\}/,
    );
  });
});
