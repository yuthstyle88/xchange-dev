import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { CashPolicy } from './CashPolicy';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { th as screenTh, en as screenEn } from './i18n';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type { CashTransferPolicyData } from '../../../shared/api/generated/cash_transfer_policy';

const SAMPLE_POLICY: CashTransferPolicyData = {
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
};

const SAMPLE_CASH: AdminCashData = {
  shift: null,
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD', 'EUR'],
  lastCounted: null,
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let viewHandlers: Record<string, (params?: URLSearchParams) => Promise<Response>>;
let saveHandler: (body: unknown) => Promise<Response>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubServer(overrides: {
  views?: Record<string, (params?: URLSearchParams) => Promise<Response>>;
  save?: (body: unknown) => Promise<Response>;
} = {}) {
  viewHandlers = {
    cash_transfer_policy: async () => json(200, { version: 1, changed: true, data: SAMPLE_POLICY }),
    admin_cash: async () => json(200, { version: 1, changed: true, data: SAMPLE_CASH }),
    ...overrides.views,
  };
  saveHandler = overrides.save ?? (async () => json(200, { version: 1, changed: true, data: SAMPLE_POLICY }));

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const [path, qs] = url.split('?');
      const params = new URLSearchParams(qs ?? '');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && path!.startsWith('/api/v3/views/')) {
        const viewName = path!.slice('/api/v3/views/'.length);
        if (viewHandlers[viewName]) {
          return viewHandlers[viewName]!(params);
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }

      if (method === 'PUT' && path === '/api/cash-transfer-policy') {
        const body = init?.body ? JSON.parse(String(init.body)) : {};
        return saveHandler(body);
      }

      return json(200, {});
    }),
  );
}

function mount(branchSignal?: () => string) {
  const [defaultBranch] = createSignal('MAIN');
  const b = branchSignal ?? defaultBranch;
  const [p] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [s] = createSignal(true);
  dispose = render(() => <CashPolicy branch={b} period={p} shown={s} />, host);
}

function getAlertText(): string | null {
  const el = host.querySelector('[role="alert"]');
  return el ? el.textContent?.trim() ?? '' : null;
}

describe('WE-9 cash-policy wordedError (#1758)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  // ── Catch 1: readPolicy ────────────────────────────────────────────────

  describe('readPolicy catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      stubServer({
        views: {
          cash_transfer_policy: async () => {
            throw new TypeError('Failed to fetch');
          },
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.network']);

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      stubServer({
        views: {
          cash_transfer_policy: async () =>
            json(500, { error: { code: 'roleNotAllowed', message: 'Internal Server Error' } }),
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.serverError']);
    });

    it('(3) refusal with unkeyed code shows server message (W10 point 4)', async () => {
      const serverMsg = 'นโยบายถูกระงับการใช้งานโดยผู้ดูแลระบบ';
      stubServer({
        views: {
          cash_transfer_policy: async () =>
            json(400, { error: { code: 'unkeyedPolicyCode', message: serverMsg } }),
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(serverMsg);
    });
  });

  // ── Catch 2: readRegister ──────────────────────────────────────────────

  describe('readRegister catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      stubServer({
        views: {
          admin_cash: async () => {
            throw new TypeError('Failed to fetch');
          },
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.network']);

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      stubServer({
        views: {
          admin_cash: async () =>
            json(500, { error: { code: 'roleNotAllowed', message: 'Internal Server Error' } }),
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.serverError']);
    });

    it('(3) refusal with unkeyed code shows server message (W10 point 4)', async () => {
      const serverMsg = 'ไม่พบข้อมูลเงินสดของสาขานี้';
      stubServer({
        views: {
          admin_cash: async () =>
            json(400, { error: { code: 'unkeyedCashCode', message: serverMsg } }),
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(serverMsg);
    });
  });

  // ── Catch 3: save ──────────────────────────────────────────────────────

  describe('save catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      stubServer({
        save: async () => {
          throw new TypeError('Failed to fetch');
        },
      });

      mount();
      await vi.waitFor(() => expect(host.querySelector('input[data-currency="USD"]')).not.toBeNull());

      const input = host.querySelector<HTMLInputElement>('input[data-currency="USD"]')!;
      input.value = '400.00';
      input.dispatchEvent(new Event('input', { bubbles: true }));

      const saveBtn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
      saveBtn.click();

      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.network']);

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await vi.waitFor(() => expect(host.querySelector('input[data-currency="USD"]')).not.toBeNull());
      const inputEn = host.querySelector<HTMLInputElement>('input[data-currency="USD"]')!;
      inputEn.value = '400.00';
      inputEn.dispatchEvent(new Event('input', { bubbles: true }));
      const saveBtnEn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
      saveBtnEn.click();

      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      stubServer({
        save: async () =>
          json(500, { error: { code: 'roleNotAllowed', message: 'Internal Server Error' } }),
      });

      mount();
      await vi.waitFor(() => expect(host.querySelector('input[data-currency="USD"]')).not.toBeNull());

      const saveBtn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
      saveBtn.click();

      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.serverError']);
    });

    it('(3) refusal with unkeyed code shows server message (W10 point 4)', async () => {
      const serverMsg = 'วงเงินต้องเป็นจำนวนบวกเท่านั้น';
      stubServer({
        save: async () =>
          json(400, { error: { code: 'unkeyedSaveCode', message: serverMsg } }),
      });

      mount();
      await vi.waitFor(() => expect(host.querySelector('input[data-currency="USD"]')).not.toBeNull());

      const saveBtn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
      saveBtn.click();

      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(serverMsg);
    });
  });

  // ── WE4 Screen-specific text for roleNotAllowedHere & idempotencyConflict ──

  describe('WE4 screen-specific wording', () => {
    it.each(['roleNotAllowedHere', 'idempotencyConflict'] as const)(
      '%s uses screen-specific sentence in TH and EN',
      async (code) => {
        stubServer({
          save: async () =>
            json(409, { error: { code, message: 'generic wire message' } }),
        });

        mount();
        await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());
        const saveBtn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
        saveBtn.click();

        await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
        expect(getAlertText()).toBe(screenTh[`error.${code}`]);

        // In EN
        dispose?.();
        dispose = undefined;
        host.innerHTML = '';
        setLang('en');

        mount();
        await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());
        const saveBtnEn = host.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
        saveBtnEn.click();

        await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
        expect(getAlertText()).toBe(screenEn[`error.${code}`]);
      },
    );

    it('500 with screen-specific code roleNotAllowedHere shows error.serverError (WE2 (a))', async () => {
      stubServer({
        views: {
          cash_transfer_policy: async () =>
            json(500, { error: { code: 'roleNotAllowedHere', message: 'Internal Server Error' } }),
        },
      });

      mount();
      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.serverError']);
    });
  });

  // ── §4.3: Branch switch stale data handling ────────────────────────────

  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const [branch, setBranch] = createSignal('MAIN');
      stubServer();

      mount(() => branch());
      await vi.waitFor(() => expect(host.querySelector('input[data-currency="THB"]')).not.toBeNull());

      // MAIN loaded currencies successfully (THB is only in register, not in policy limits)
      expect(host.querySelector('input[data-currency="THB"]')).not.toBeNull();

      // Switch to B02, where readRegister fails with 500
      stubServer({
        views: {
          admin_cash: async () =>
            json(500, { error: { code: 'roleNotAllowed', message: 'Failed to read B02' } }),
        },
      });
      setBranch('B02');

      await vi.waitFor(() => expect(getAlertText()).not.toBeNull());
      expect(getAlertText()).toBe(th['error.serverError']);

      // Previous branch currencies from register must NOT stay on screen (setRegister([]) at CashPolicy.tsx:76)
      expect(host.querySelector('input[data-currency="THB"]')).toBeNull();
    });
  });
});

