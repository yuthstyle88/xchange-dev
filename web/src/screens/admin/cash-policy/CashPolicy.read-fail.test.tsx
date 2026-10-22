// R3-2 (docs/design/web-silent-failures.md §7.2): a failed `cash_transfer_policy` read must never
// let Save write a new org policy over limits the screen never showed. Up-front tests: rows (a),
// (b) and (d) are red until `CashPolicy.tsx` keeps a `known` state; row (c) is a guard.
// The worded refusal alert stays exactly as today and stays the only `role="alert"` (pinned by
// `worded.test.tsx` and `CashPolicy.branch-register.test.tsx` P5); `unreadable` is plain banner text
// in place of "No limit set yet", with a `type="button"` retry beside it (§7.2 at 01e980a6).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { CashPolicy } from './CashPolicy';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { makeAdminCashData, makeCashTransferPolicyData, type CashTransferPolicyData } from '../../../shared/testing/fixtures';
import * as texts from './i18n';

const POLICY = makeCashTransferPolicyData({
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
});

const CASH = makeAdminCashData({
  openShifts: [],
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD', 'EUR'],
  lastCounted: null,
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  /** The policy read's answer: the data (or null for none), or a failure status. */
  policy: CashTransferPolicyData | null | { fail: number } = POLICY;
  policyReads = 0;
  saves: unknown[] = [];

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET' && u.pathname === '/api/v3/views/cash_transfer_policy') {
      this.policyReads++;
      const p = this.policy;
      if (p && 'fail' in p) return json(p.fail, { error: { code: 'serverError', message: 'policy failed' } });
      return view(p);
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/admin_cash') return view(CASH);
    if (method === 'PUT' && u.pathname === '/api/cash-transfer-policy') {
      this.saves.push(init?.body ? JSON.parse(String(init.body)) : null);
      return view(POLICY);
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
  };
}

let server: Server;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let setBranchSignal: Setter<string>;

async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(): Promise<HTMLElement> {
  const [branch, setBranch] = createSignal('MAIN');
  setBranchSignal = setBranch;
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  dispose = render(() => <CashPolicy branch={branch} period={period} shown={shown} />, host);
  await settle();
  return host.querySelector<HTMLElement>('[data-section="cash-policy"]')!;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(async () => {
  await settle();
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const saveButton = (s: ParentNode) => s.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
const limitInput = (s: ParentNode, cur: string) => s.querySelector<HTMLInputElement>(`input[data-currency="${cur}"]`);
const banner = (s: ParentNode) => s.querySelector<HTMLElement>('.stock-banner');
const alerts = (s: ParentNode) => Array.from(s.querySelectorAll('[role="alert"]'));
const retryButton = (s: ParentNode) =>
  Array.from(s.querySelectorAll<HTMLButtonElement>('button[type="button"]')).find(
    (b) => text(b) === texts.th.retry,
  ) ?? null;

/** Submits the form directly, as Enter in an input would, bypassing a disabled Save button. */
async function submitForm(s: HTMLElement): Promise<void> {
  s.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await settle();
}

describe('CashPolicy read failure (R3-2)', () => {
  it('(a) a failed policy read shows unreadable in the banner with a retry, keeps the worded alert, no "No limit set yet", and Save is disabled and sends nothing', async () => {
    server.policy = { fail: 500 };
    const s = await mount();

    expect(text(banner(s)), 'the banner reads unreadable').toContain(texts.th.unreadable);
    expect(retryButton(s), 'a type="button" retry').not.toBeNull();
    expect(text(s)).not.toContain(texts.th.noLimit);
    expect(alerts(s).map(text), 'the worded refusal stays the only alert').toEqual([sharedTh['error.serverError']]);
    expect(saveButton(s).disabled).toBe(true);
    expect(saveButton(s).title).toBe(texts.th.unreadable);

    const usd = limitInput(s, 'USD')!;
    usd.value = '100';
    usd.dispatchEvent(new Event('input', { bubbles: true }));
    await submitForm(s);
    expect(server.saves).toHaveLength(0);
  });

  it('(b) a successful retry removes the alert, fills in the limits and enables Save', async () => {
    server.policy = { fail: 500 };
    const s = await mount();
    const retry = retryButton(s);
    expect(retry, 'a type="button" retry').not.toBeNull();

    server.policy = POLICY;
    retry!.click();
    await settle();

    expect(server.policyReads).toBe(2);
    expect(text(s)).not.toContain(texts.th.unreadable);
    expect(text(banner(s))).toContain('v-0001');
    expect(limitInput(s, 'USD')?.value).toBe('300.00');
    expect(limitInput(s, 'EUR')?.value).toBe('12.5');
    expect(saveButton(s).disabled).toBe(false);
  });

  it('(c) guard: a successful read answering none shows "No limit set yet" and Save is enabled', async () => {
    server.policy = null;
    const s = await mount();

    expect(text(banner(s))).toContain(texts.th.noLimit);
    expect(saveButton(s).disabled).toBe(false);
    expect(text(s)).not.toContain(texts.th.unreadable);
    expect(retryButton(s)).toBeNull();
  });

  it('(d) P5 still holds: after a failed policy read, a branch switch keeps the worded alert and the unreadable banner, and Save stays disabled', async () => {
    server.policy = { fail: 500 };
    const s = await mount();

    setBranchSignal('B02');
    await settle();

    expect(limitInput(s, 'THB'), 'the new branch register was read').not.toBeNull();
    expect(alerts(s).map(text), 'the worded policy alert stays').toEqual([sharedTh['error.serverError']]);
    expect(text(banner(s)), 'the unreadable banner stays').toContain(texts.th.unreadable);
    expect(saveButton(s).disabled).toBe(true);
    expect(server.saves).toHaveLength(0);
  });
});
