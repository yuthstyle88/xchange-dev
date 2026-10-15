// KW-3 (#2021): idempotency-key lifecycle tests for Reconciliation (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Toast } from '../parts/toast';
import { Reconciliation } from './Reconciliation';
import * as table from './i18n';

const B01 = 'XC-B01';
const VIEW = '/api/v3/views/finance_reconciliation';
const CERTIFY = '/api/v3/commands/reconciliation_certify';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: Reconciliation idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

  const sheet = {
    branch: B01,
    date: '2026-10-15',
    transactions: {
      available: true,
      reason: null,
      data: {
        byMethod: [{ method: 'Cash', buyThb: '1000.00', sellThb: '2000.00', fees: '0.00', bankFees: '0.00', count: 1 }],
        reversedThb: '0.00',
      },
    },
    counts: { available: false, reason: 'CA-3', data: null },
    transfers: { available: false, reason: 'CA-5', data: null },
    exceptions: { available: true, reason: null, data: [] },
    integrity: { available: false, reason: 'LG-7', data: null },
    reversalsPending: { available: false, reason: 'RV-1', data: null },
    figuresHash: 'hash-abc-123',
    certifications: [],
  };

  const certResponse = {
    id: 'C-1',
    certifiedBy: 'u-fin',
    certifiedAt: '2026-10-15T10:00:00Z',
    note: 'ok',
    figuresHash: 'hash-abc-123',
    stale: false,
  };

  beforeEach(() => {
    setLang('th');
    writes = [];
    queue = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);

      if (method === 'GET' && u.pathname === VIEW) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(sheet) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'POST' && u.pathname === CERTIFY) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: structuredClone(certResponse) };
        if (step === 'network') {
          throw new TypeError('Failed to fetch');
        }
        return new Response(JSON.stringify(step.body), {
          status: step.status,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <Reconciliation branch={branch} period={period} shown={shown} />
          <Toast />
        </>
      ),
      root,
    );
    await settle();
  }

  function certifyButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.certify,
    );
    expect(btn, 'certify button exists').toBeDefined();
    return btn!;
  }

  function noteInput(): HTMLInputElement {
    const input = root.querySelector('.reconciliation-certify input[type="text"]') as HTMLInputElement | null;
    expect(input, 'note input exists').not.toBeNull();
    return input!;
  }

  async function clickCertify(): Promise<void> {
    certifyButton().click();
    await settle();
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountScreen();
    setInputValue(noteInput(), 'day checked');
    queue.push('network', { status: 200, body: structuredClone(certResponse) });

    await clickCertify();
    await clickCertify();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountScreen();
    setInputValue(noteInput(), 'day checked');
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: structuredClone(certResponse) },
    );

    await clickCertify();
    await clickCertify();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountScreen();
    setInputValue(noteInput(), 'day checked');
    queue.push(
      { status: 409, body: { error: { code: 'staleVersion', message: 'stale figures' } } },
      { status: 200, body: structuredClone(certResponse) },
    );

    await clickCertify();
    await clickCertify();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountScreen();
    setInputValue(noteInput(), 'first note');
    queue.push('network', { status: 200, body: structuredClone(certResponse) });

    await clickCertify();
    setInputValue(noteInput(), 'updated note');
    await clickCertify();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountScreen();
    setInputValue(noteInput(), 'day checked');
    queue.push(
      { status: 200, body: structuredClone(certResponse) },
      { status: 200, body: structuredClone(certResponse) },
    );

    await clickCertify();
    setInputValue(noteInput(), 'day checked');
    await clickCertify();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
