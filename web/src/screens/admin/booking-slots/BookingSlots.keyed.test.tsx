// KW-3 (#2021): idempotency-key lifecycle tests for BookingSlots (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const B01 = 'XC-B01';
const VIEW = '/api/v3/views/admin_booking_slots';
const SAVE_RULE = `/api/branches/${B01}/booking-slot-rule`;

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: BookingSlots idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

  const sampleView = {
    rule: {
      branchId: B01,
      slotMinutes: 30,
      capacity: 8,
      horizonDays: 21,
    },
    closures: [],
    slots: [],
    policy: {
      cancelCutoffMinutes: 45,
      noShowGraceMinutes: 20,
      confirmExpiryMinutes: 30,
    },
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
          JSON.stringify({ version: 1, changed: true, data: structuredClone(sampleView) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'PUT' && u.pathname === SAVE_RULE) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: {} };
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
    dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, root);
    await settle();
  }

  function saveRuleButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.saveRule,
    );
    expect(btn, 'saveRule button exists').toBeDefined();
    return btn!;
  }

  function capacityInput(): HTMLInputElement {
    const input = root.querySelector('input[name="capacity"]') as HTMLInputElement | null;
    expect(input, 'capacity input exists').not.toBeNull();
    return input!;
  }

  async function clickSaveRule(): Promise<void> {
    saveRuleButton().click();
    await settle();
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountScreen();
    setInputValue(capacityInput(), '9');
    queue.push('network', { status: 200, body: {} });

    await clickSaveRule();
    await clickSaveRule();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountScreen();
    setInputValue(capacityInput(), '10');
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: {} },
    );

    await clickSaveRule();
    await clickSaveRule();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountScreen();
    setInputValue(capacityInput(), '11');
    queue.push(
      { status: 400, body: { error: { code: 'invalidSlotRule', message: 'invalid rule' } } },
      { status: 200, body: {} },
    );

    await clickSaveRule();
    await clickSaveRule();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountScreen();
    setInputValue(capacityInput(), '12');
    queue.push('network', { status: 200, body: {} });

    await clickSaveRule();
    setInputValue(capacityInput(), '14');
    await clickSaveRule();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountScreen();
    setInputValue(capacityInput(), '8');
    queue.push({ status: 200, body: {} }, { status: 200, body: {} });

    await clickSaveRule();
    await clickSaveRule();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
