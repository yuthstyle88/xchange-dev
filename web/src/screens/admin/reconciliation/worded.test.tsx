import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { Reconciliation } from './Reconciliation';
import {
  makeCertificationRow,
  makeFinanceSheet,
  makeMethodTotals,
  makePart,
  makeTransactionTotals,
} from '../../../shared/testing/fixtures';

const B01 = 'XC-B01';
const B02 = 'XC-B02';

function sampleSheet(branch: string, date: string) {
  return makeFinanceSheet({
    branch,
    date,
    transactions: makePart(
      makeTransactionTotals({
        byMethod: [
          makeMethodTotals({ method: 'Cash', buyThb: '1000.00', sellThb: '2000.00', fees: '10.00', bankFees: '0.00', count: 2 }),
        ],
        reversedThb: '0.00',
      }),
      { available: true, reason: null },
    ),
    counts: makePart<never>(null, { available: false, reason: 'CA-3' }),
    transfers: makePart<never>(null, { available: false, reason: 'CA-5' }),
    exceptions: makePart([], { available: false, reason: null }),
    integrity: makePart([], { available: false, reason: null }),
    reversalsPending: makePart([], { available: false, reason: null }),
    figuresHash: `hash-${branch}-123`,
    certifications: [],
  });
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Reconciliation wordedError (#1771)', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;
  let viewHandler: (branch: string, date: string) => Promise<Response>;
  let certifyHandler: (body: any) => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    viewHandler = async (branch, date) =>
      jsonResponse(200, { version: 1, changed: true, data: sampleSheet(branch, date) });
    certifyHandler = async () =>
      jsonResponse(
        200,
        makeCertificationRow({
          id: 'C-1',
          certifiedBy: 'u-fin',
          certifiedAt: '2026-10-15T00:00:00Z',
          note: '',
          figuresHash: 'hash-XC-B01-123',
          stale: false,
        }),
      );

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const urlStr = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(urlStr, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/finance_reconciliation' && method === 'GET') {
        const branch = url.searchParams.get('branch') ?? '';
        const date = url.searchParams.get('date') ?? '';
        return viewHandler(branch, date);
      }

      if (url.pathname === '/api/v3/commands/reconciliation_certify' && method === 'POST') {
        const body = typeof init?.body === 'string' && init.body ? JSON.parse(init.body) : {};
        return certifyHandler(body);
      }

      return jsonResponse(200, { version: 1, changed: false });
    }));
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.unstubAllGlobals();
    setLang('th');
  });

  function mount(initialBranch = B01) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown, setShown] = createSignal(true);
    const el = document.createElement('div');
    root = el;
    document.body.appendChild(el);
    dispose = render(
      () => (<Reconciliation branch={branch} period={period} shown={shown} />) as JSX.Element,
      el,
    );
    return { el, setBranch, setShown };
  }

  function errorText(): string {
    const el = root?.querySelector('.reconciliation-error, [role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
    for (let i = 0; i < 5; i++) await Promise.resolve();
  }

  // ── Catch 1: read (finance_reconciliation view) ───────────────────────────
  describe('Catch 1: read (finance_reconciliation view)', () => {
    it('(1) network TypeError shows error.network and reacts to setLang without remounting', async () => {
      viewHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(errorText()).toBe(sharedTh['error.network']);

      setLang('en');
      await settle();
      expect(errorText()).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      viewHandler = async () =>
        jsonResponse(500, {
          error: { code: 'roleNotAllowedHere', message: 'role not allowed on this branch' },
        });
      mount();
      await settle();

      expect(errorText()).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      viewHandler = async () =>
        jsonResponse(400, {
          error: {
            code: 'unmappedFinanceSheetRefusal',
            message: 'ข้อความปฏิเสธเฉพาะกิจของใบกระทบยอด',
          },
        });
      mount();
      await settle();

      expect(errorText()).toBe('ข้อความปฏิเสธเฉพาะกิจของใบกระทบยอด');
    });

    it('stale data from earlier read is cleared after a branch switch fails', async () => {
      viewHandler = async (branch, date) => {
        if (branch === B02) {
          throw new TypeError('Failed to fetch B02');
        }
        return jsonResponse(200, { version: 1, changed: true, data: sampleSheet(branch, date) });
      };

      const { setBranch } = mount(B01);
      await settle();

      // Successful read shows transactions section
      expect(root?.querySelector('[data-part="transactions"]')).not.toBeNull();
      expect(errorText()).toBe('');

      // Switch to B02 which fails
      setBranch(B02);
      await settle();

      // Sheet from B01 must not stay on screen
      expect(root?.querySelector('[data-part="transactions"]')).toBeNull();
      expect(errorText()).toBe(sharedTh['error.network']);
    });
  });

  // ── Catch 2: certify (reconciliation_certify command) ─────────────────────
  describe('Catch 2: certify (reconciliation_certify command)', () => {
    async function triggerCertify(el: HTMLElement) {
      const btn = Array.from(el.querySelectorAll<HTMLButtonElement>('button')).find((b) =>
        (b.textContent ?? '').includes('รับรอง') || (b.textContent ?? '').includes('Certify') || (b.textContent ?? '').includes('certify'),
      );
      expect(btn).toBeDefined();
      btn!.click();
      await settle();
    }

    it('(1) network TypeError shows error.network and reacts to setLang without remounting', async () => {
      const { el } = mount();
      await settle();

      certifyHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await triggerCertify(el);

      expect(errorText()).toBe(sharedTh['error.network']);

      setLang('en');
      await settle();
      expect(errorText()).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      const { el } = mount();
      await settle();

      certifyHandler = async () =>
        jsonResponse(500, {
          error: { code: 'roleNotAllowedHere', message: 'role not allowed to certify' },
        });
      await triggerCertify(el);

      expect(errorText()).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const { el } = mount();
      await settle();

      certifyHandler = async () =>
        jsonResponse(400, {
          error: {
            code: 'unmappedCertifyRefusal',
            message: 'ไม่สามารถรับรองยอดได้เนื่องจากรายการขัดข้อง',
          },
        });
      await triggerCertify(el);

      expect(errorText()).toBe('ไม่สามารถรับรองยอดได้เนื่องจากรายการขัดข้อง');
    });
  });
});
