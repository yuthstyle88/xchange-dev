import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { en } from '../../../../shared/i18n/en';
import { th } from '../../../../shared/i18n/th';
import { ProfitSummaryPanel } from './ProfitSummaryPanel';
import { ShiftValuationPanel } from './ShiftValuationPanel';

type Reply = { status: number; body?: unknown; reject?: Error };

const VALUATION = '/api/v3/compute/shift_valuation';

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function text(el: Element | null | undefined): string {
  return el?.textContent?.replace(/\s+/g, ' ').trim() ?? '';
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('WE-11: calculators worded error tests (web-worded-error.md)', () => {
  let replyHandler: (url: string, body: Record<string, unknown>) => Reply;
  let root: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    replyHandler = () => ({ status: 200, body: {} });
    vi.stubGlobal('fetch', vi.fn(async (input: string, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
      const rep = replyHandler(url.pathname, body);
      if (rep.reject) {
        throw rep.reject;
      }
      return new Response(JSON.stringify(rep.body ?? {}), {
        status: rep.status,
        headers: { 'content-type': 'application/json' },
      });
    }));
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.unstubAllGlobals();
    setLang('th');
  });

  function mountProfit(branchSignal?: () => string): HTMLDivElement {
    root = document.createElement('div');
    document.body.appendChild(root);
    const api = new ApiClient();
    dispose = render(
      () => <ProfitSummaryPanel api={api} branch={branchSignal ? branchSignal() : 'B1'} shiftId="S-1" />,
      root,
    );
    return root;
  }

  function mountValuation(branchSignal?: () => string): HTMLDivElement {
    root = document.createElement('div');
    document.body.appendChild(root);
    const api = new ApiClient();
    dispose = render(
      () => <ShiftValuationPanel api={api} branch={branchSignal ? branchSignal() : 'B1'} shiftId="S-1" />,
      root,
    );
    return root;
  }

  async function submitProfitForm(container: HTMLElement): Promise<void> {
    const inputs = container.querySelectorAll<HTMLInputElement>('input[type="number"]');
    inputs.forEach((inp) => {
      inp.value = '100';
      inp.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const chk = container.querySelector<HTMLInputElement>('input[type="checkbox"]');
    if (chk && !chk.checked) {
      chk.click();
    }
    const btn = container.querySelector<HTMLButtonElement>('button.primary');
    btn?.click();
    await settle();
  }

  async function submitValuationForm(container: HTMLElement): Promise<void> {
    const dateInput = container.querySelector<HTMLInputElement>('input[type="date"]');
    if (dateInput) {
      dateInput.value = '2026-10-15';
      dateInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const numInputs = container.querySelectorAll<HTMLInputElement>('input[type="number"]');
    numInputs.forEach((inp) => {
      inp.value = '35';
      inp.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const btn = container.querySelector<HTMLButtonElement>('button.primary');
    btn?.click();
    await settle();
  }

  // ── ProfitSummaryPanel: submit catch ──────────────────────────────────────

  describe('ProfitSummaryPanel submit catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replyHandler = () => ({ status: 0, reject: new TypeError('Failed to fetch') });
      const el = mountProfit();
      await settle();

      await submitProfitForm(el);
      const alert = el.querySelector('[role="alert"]');
      expect(text(alert)).toBe(th['error.network']);
      expect(text(alert)).not.toContain('Failed to fetch');

      setLang('en');
      await submitProfitForm(el);
      expect(text(alert)).toBe(en['error.network']);
      expect(text(alert)).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      replyHandler = () => refusal(500, 'upstreamDatabaseCrash', 'raw upstream crash message');
      const el = mountProfit();
      await settle();

      await submitProfitForm(el);
      const alert = el.querySelector('[role="alert"]');
      expect(text(alert)).toBe(th['error.serverError']);
      expect(text(alert)).not.toContain('raw upstream crash message');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'กำไรที่ระบุไม่ตรงกับยอดปิดกะ';
      replyHandler = () => refusal(400, 'unmappedProfitCode', sentence);
      const el = mountProfit();
      await settle();

      await submitProfitForm(el);
      const alert = el.querySelector('[role="alert"]');
      expect(text(alert)).toBe(sentence);
      expect(text(alert)).not.toBe(th['error.serverError']);
    });
  });

  // ── ShiftValuationPanel: list catch ───────────────────────────────────────

  describe('ShiftValuationPanel list catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replyHandler = (url) => {
        if (url === VALUATION) {
          return { status: 0, reject: new TypeError('Failed to fetch') };
        }
        return { status: 200, body: {} };
      };
      const el = mountValuation();
      await settle();

      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(th['error.network']);
      expect(text(live)).not.toContain('Failed to fetch');

      setLang('en');
      // Trigger re-read / remount in en
      dispose?.();
      mountValuation();
      await settle();
      const liveEn = root!.querySelector('[aria-live="polite"]');
      expect(text(liveEn)).toBe(en['error.network']);
      expect(text(liveEn)).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      replyHandler = (url) => {
        if (url === VALUATION) {
          return refusal(500, 'listServerDown', 'database offline');
        }
        return { status: 200, body: {} };
      };
      const el = mountValuation();
      await settle();

      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(th['error.serverError']);
      expect(text(live)).not.toContain('database offline');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถอ่านเรทของกะนี้ได้';
      replyHandler = (url) => {
        if (url === VALUATION) {
          return refusal(400, 'unmappedListCode', sentence);
        }
        return { status: 200, body: {} };
      };
      const el = mountValuation();
      await settle();

      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(sentence);
      expect(text(live)).not.toBe(th['error.serverError']);
    });
  });

  // ── ShiftValuationPanel: submit catch ─────────────────────────────────────

  describe('ShiftValuationPanel submit catch', () => {
    const mockCurrencies = [
      { currency: 'USD', opening: '100.00', current: '200.00', needsRate: true },
    ];

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replyHandler = (url, body) => {
        if (url === VALUATION) {
          if (body.rates == null) {
            return { status: 200, body: { currencies: mockCurrencies } };
          }
          return { status: 0, reject: new TypeError('Failed to fetch') };
        }
        return { status: 200, body: {} };
      };

      const el = mountValuation();
      await settle();

      await submitValuationForm(el);
      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(th['error.network']);
      expect(text(live)).not.toContain('Failed to fetch');

      setLang('en');
      await submitValuationForm(el);
      expect(text(live)).toBe(en['error.network']);
      expect(text(live)).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      replyHandler = (url, body) => {
        if (url === VALUATION) {
          if (body.rates == null) {
            return { status: 200, body: { currencies: mockCurrencies } };
          }
          return refusal(500, 'computeServerDown', 'internal execution failure');
        }
        return { status: 200, body: {} };
      };

      const el = mountValuation();
      await settle();

      await submitValuationForm(el);
      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(th['error.serverError']);
      expect(text(live)).not.toContain('internal execution failure');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'เรทส่วนต่างไม่สอดคล้องกับตลาด';
      replyHandler = (url, body) => {
        if (url === VALUATION) {
          if (body.rates == null) {
            return { status: 200, body: { currencies: mockCurrencies } };
          }
          return refusal(400, 'unmappedValuationCode', sentence);
        }
        return { status: 200, body: {} };
      };

      const el = mountValuation();
      await settle();

      await submitValuationForm(el);
      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(sentence);
      expect(text(live)).not.toBe(th['error.serverError']);
    });
  });

  // ── Done Criterion 3: branch switch and stale data ───────────────────────

  describe('Done Criterion 3: stale data dropped after branch switch', () => {
    it('ShiftValuationPanel: failed read on branch switch clears previous branch currencies', async () => {
      const [branch, setBranch] = createSignal('B1');
      replyHandler = (url, body) => {
        if (url === VALUATION) {
          if (body.branch === 'B1') {
            return {
              status: 200,
              body: {
                currencies: [{ currency: 'USD', opening: '100.00', current: '200.00', needsRate: true }],
              },
            };
          }
          return refusal(500, 'serverError', 'Branch B2 load error');
        }
        return { status: 200, body: {} };
      };

      const el = mountValuation(branch);
      await settle();

      // Branch B1 shows USD rate input
      expect(el.querySelector('input[type="number"]')).not.toBeNull();

      // Switch to B2 where read fails
      setBranch('B2');
      await settle();

      // Error shown, and previous USD input is NOT left on screen
      const live = el.querySelector('[aria-live="polite"]');
      expect(text(live)).toBe(th['error.serverError']);
      expect(el.querySelector('input[type="number"]')).toBeNull();
    });

    it('ProfitSummaryPanel: switching branch clears previous computed result cards', async () => {
      const [branch, setBranch] = createSignal('B1');
      replyHandler = () => ({
        status: 200,
        body: { realized: '50000.00', unrealized: '1200.00', withdrawable: '40000.00' },
      });

      const el = mountProfit(branch);
      await settle();

      await submitProfitForm(el);
      expect(text(el)).toContain('50,000.00 THB');

      // Switch branch to B2
      setBranch('B2');
      await settle();

      // Result should be cleared, returning to รอข้อมูล
      expect(text(el)).not.toContain('50,000.00 THB');
      expect(text(el)).toContain('รอข้อมูล');
    });
  });
});
