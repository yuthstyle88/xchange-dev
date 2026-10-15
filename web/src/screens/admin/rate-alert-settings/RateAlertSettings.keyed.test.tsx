// KW-3 (#2021): idempotency-key lifecycle tests for RateAlertSettings (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { RateAlertSettings } from './RateAlertSettings';

const VIEW = '/api/v3/views/rate_alert_settings';
const SAVE = '/api/rate-alert-settings';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: RateAlertSettings idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

  const initialData = {
    defaultCooldownSeconds: 86400,
    maxActivePerCustomer: 10,
    defaultLifetimeDays: 30,
    observePolicy: 'ALL_FRESH' as const,
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
          JSON.stringify({ version: 1, changed: true, data: structuredClone(initialData) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'PUT' && u.pathname === SAVE) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: structuredClone(initialData) };
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
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <RateAlertSettings branch={branch} period={period} shown={shown} />, root);
    await settle();
  }

  function saveButton(): HTMLButtonElement {
    const btn = root.querySelector('button.rate-alert-submit-btn') as HTMLButtonElement | null;
    expect(btn, 'save button exists').not.toBeNull();
    return btn!;
  }

  function maxActiveInput(): HTMLInputElement {
    const input = root.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement | null;
    expect(input, 'maxActivePerCustomer input exists').not.toBeNull();
    return input!;
  }

  async function clickSave(): Promise<void> {
    saveButton().click();
    await settle();
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountScreen();
    queue.push('network', { status: 200, body: structuredClone(initialData) });

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountScreen();
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: structuredClone(initialData) },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountScreen();
    queue.push(
      { status: 400, body: { error: { code: 'validationFailed', message: 'bad limit' } } },
      { status: 200, body: structuredClone(initialData) },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountScreen();
    queue.push('network', {
      status: 200,
      body: { ...initialData, maxActivePerCustomer: 15 },
    });

    await clickSave();
    setInputValue(maxActiveInput(), '15');
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountScreen();
    queue.push(
      { status: 200, body: structuredClone(initialData) },
      { status: 200, body: structuredClone(initialData) },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
