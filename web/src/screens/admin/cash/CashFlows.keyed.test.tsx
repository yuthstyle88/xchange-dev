// KW-3 (#2021): idempotency-key lifecycle tests for CashFlows (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

const CREATE_TRANSFER = '/api/cash-transfers';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: CashFlows idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

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

      if (method === 'GET') {
        if (u.pathname === '/api/v3/views/me') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                id: 'u-admin',
                name: 'Admin',
                roles: ['ADMIN'],
                permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
                branch: 'MAIN',
                branchName: 'Headquarters',
                branches: [{ id: 'MAIN', code: 'MAIN', name: 'Headquarters' }],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/cash_counts') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/cash_transfers') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/denominations') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: { rows: [{ id: 1, currency: 'THB', faceValue: '1000', kind: 'NOTE' }] },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/transfer_branches') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: { rows: [] } }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (method === 'POST' && u.pathname === CREATE_TRANSFER) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: { id: 'tr-1', status: 'DRAFT' } };
        if (step === 'network') {
          throw new TypeError('Failed to fetch');
        }
        return new Response(JSON.stringify(step.body), {
          status: step.status,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    closeDialog();
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
    const [branch] = createSignal('MAIN');
    const [shown] = createSignal(true);
    const [shifts] = createSignal([]);
    dispose = render(
      () => (
        <>
          <CashFlows branch={branch} shown={shown} shifts={shifts} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    await settle();
  }

  function kindSelect(): HTMLSelectElement {
    const select = root.querySelector('#transfer-kind') as HTMLSelectElement | null;
    expect(select, 'transfer-kind select exists').not.toBeNull();
    return select!;
  }

  function noteInput(): HTMLInputElement {
    const input = root.querySelector('#transfer-note') as HTMLInputElement | null;
    expect(input, 'transfer-note input exists').not.toBeNull();
    return input!;
  }

  function createButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-action="create"]') as HTMLButtonElement | null;
    expect(btn, 'create button exists').not.toBeNull();
    return btn!;
  }

  function setSelectValue(select: HTMLSelectElement, value: string): void {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function clickCreate(): Promise<void> {
    createButton().click();
    await settle();
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountScreen();
    setSelectValue(kindSelect(), 'EXTERNAL_OUT');
    setInputValue(noteInput(), 'bank deposit');
    queue.push('network', { status: 200, body: { id: 'tr-1', status: 'DRAFT' } });

    await clickCreate();
    await clickCreate();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountScreen();
    setSelectValue(kindSelect(), 'EXTERNAL_OUT');
    setInputValue(noteInput(), 'bank deposit');
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: { id: 'tr-1', status: 'DRAFT' } },
    );

    await clickCreate();
    await clickCreate();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountScreen();
    setSelectValue(kindSelect(), 'EXTERNAL_OUT');
    setInputValue(noteInput(), 'bank deposit');
    queue.push(
      { status: 400, body: { error: { code: 'validationFailed', message: 'empty lines' } } },
      { status: 200, body: { id: 'tr-1', status: 'DRAFT' } },
    );

    await clickCreate();
    await clickCreate();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountScreen();
    setSelectValue(kindSelect(), 'EXTERNAL_OUT');
    setInputValue(noteInput(), 'bank deposit');
    queue.push('network', { status: 200, body: { id: 'tr-1', status: 'DRAFT' } });

    await clickCreate();
    setInputValue(noteInput(), 'updated bank deposit');
    await clickCreate();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountScreen();
    setSelectValue(kindSelect(), 'EXTERNAL_OUT');
    setInputValue(noteInput(), 'bank deposit');
    queue.push(
      { status: 200, body: { id: 'tr-1', status: 'DRAFT' } },
      { status: 200, body: { id: 'tr-2', status: 'DRAFT' } },
    );

    await clickCreate();
    setInputValue(noteInput(), 'bank deposit');
    await clickCreate();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
