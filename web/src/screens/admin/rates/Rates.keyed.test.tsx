// KW-3 (#2021): idempotency-key lifecycle tests for Rates (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Rates } from './Rates';

const SAVE_POLICY = '/api/rate-policies';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: Rates idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

  const samplePolicy = {
    id: 'pol-1',
    version: 1,
    branch: 'MAIN',
    buyAdjustPct: '-0.5000',
    sellAdjustPct: '0.4000',
    guards: {},
    maxReferenceAgeHours: 24,
    onStale: 'STOP',
    staleLimitThb: null,
    reason: 'default policy',
    valueTiersOn: false,
    valueTiers: [],
    sharpMoveAlertPct: '2.50',
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

      if (method === 'GET') {
        if (u.pathname === '/api/v3/views/me') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                kind: 'owner',
                id: 'owner-1',
                role: 'OWNER',
                branch: 'MAIN',
                name: 'Owner',
                username: 'owner',
                permissions: ['OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'SystemSettings'],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/rate_policy') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: { policy: structuredClone(samplePolicy) } }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/published_rates') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                publication: { id: 'pub-1', version: 1, publishedAt: '2026-10-14T02:00:00Z' },
                sourceDate: '2026-10-14',
                stale: false,
                lines: [],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/rate_drafts') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: { drafts: [] } }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/rates_reference') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: { rows: [], fetchedAt: '2026-10-14T02:00:00Z', error: null } }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (method === 'POST' && u.pathname === SAVE_POLICY) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: { id: 'pol-2', version: 2 } };
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
    dispose = render(
      () => (
        <>
          <Rates branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    await settle();
  }

  function savePolicyButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-testid="save-policy"]') as HTMLButtonElement | null;
    expect(btn, 'save-policy button exists').not.toBeNull();
    return btn!;
  }

  function sharpMoveInput(): HTMLInputElement {
    const input = root.querySelector('input[data-testid="sharp-move-threshold"]') as HTMLInputElement | null;
    expect(input, 'sharp-move-threshold input exists').not.toBeNull();
    return input!;
  }

  async function clickSavePolicy(): Promise<void> {
    savePolicyButton().click();
    await settle();
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountScreen();
    queue.push('network', { status: 200, body: { id: 'pol-2', version: 2 } });

    await clickSavePolicy();
    await clickSavePolicy();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountScreen();
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: { id: 'pol-2', version: 2 } },
    );

    await clickSavePolicy();
    await clickSavePolicy();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountScreen();
    queue.push(
      { status: 400, body: { error: { code: 'validationFailed', message: 'invalid policy' } } },
      { status: 200, body: { id: 'pol-2', version: 2 } },
    );

    await clickSavePolicy();
    await clickSavePolicy();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountScreen();
    queue.push('network', { status: 200, body: { id: 'pol-2', version: 2 } });

    await clickSavePolicy();
    setInputValue(sharpMoveInput(), '3.75');
    await clickSavePolicy();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountScreen();
    queue.push(
      { status: 200, body: { id: 'pol-2', version: 2 } },
      { status: 200, body: { id: 'pol-3', version: 3 } },
    );

    await clickSavePolicy();
    await clickSavePolicy();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
