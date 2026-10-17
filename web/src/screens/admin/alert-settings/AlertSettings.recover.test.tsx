// Ticket #2251: AlertSettings recovery on shell Refresh (`refreshTick`) after a failed load.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { requestRefresh } from '../parts/refresh';
import { AlertSettings } from './AlertSettings';
import * as table from './i18n';

const B01 = 'XC-B01';
const VIEW = '/api/v3/views/branch_alert_settings';
const SAVE = '/api/v3/commands/branch_alert_settings_save';

type ViewReply =
  | { status: number; body: unknown }
  | 'network'
  | (() => Promise<{ status: number; body: unknown }>);

describe('AlertSettings shell refresh recovery (#2251)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let viewCalls: string[];
  let viewQueue: ViewReply[];
  let saveBodies: Array<Record<string, unknown>>;
  let saveQueue: Array<{ status: number; body: unknown }>;

  const sampleSettings = {
    branch: B01,
    staleDataMinutes: 20,
    floors: [{ currency: 'USD', minQuantity: '1500' }],
    version: 3,
  };

  beforeEach(() => {
    setLang('th');
    viewCalls = [];
    viewQueue = [];
    saveBodies = [];
    saveQueue = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const branch = u.searchParams.get('branch') ?? '';
        viewCalls.push(branch);
        const step = viewQueue.shift() ?? {
          status: 200,
          body: { version: 3, changed: true, data: structuredClone({ ...sampleSettings, branch }) },
        };
        if (step === 'network') {
          throw new TypeError('Failed to fetch');
        }
        const resolved = typeof step === 'function' ? await step() : step;
        return new Response(JSON.stringify(resolved.body), {
          status: resolved.status,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (method === 'POST' && u.pathname === SAVE) {
        const rawBody = typeof init?.body === 'string' ? init.body : '{}';
        saveBodies.push(JSON.parse(rawBody) as Record<string, unknown>);
        const reply = saveQueue.shift() ?? {
          status: 200,
          body: structuredClone(sampleSettings),
        };
        return new Response(JSON.stringify(reply.body), {
          status: reply.status,
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

  function minutesInput(): HTMLInputElement {
    const input = root.querySelector('#stale-data-minutes') as HTMLInputElement | null;
    expect(input, 'staleDataMinutes input exists').not.toBeNull();
    return input!;
  }

  function saveButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.save,
    );
    expect(btn, 'save button exists').toBeDefined();
    return btn!;
  }

  function alertBanner(): HTMLElement | null {
    return root.querySelector('[role="alert"]');
  }

  it('row 1: load fails, then the shell Refresh, load succeeds -> form filled, error gone', async () => {
    viewQueue.push(
      {
        status: 503,
        body: { error: { code: 'serviceUnavailable', message: 'unavailable' } },
      },
      {
        status: 200,
        body: { version: 3, changed: true, data: structuredClone(sampleSettings) },
      },
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);

    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();

    expect(viewCalls).toEqual([B01]);
    expect(alertBanner()?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(minutesInput().value).toBe('');
    expect(root.querySelector('[data-floor="USD"]')).toBeNull();

    requestRefresh();
    await settle();

    expect(viewCalls).toEqual([B01, B01]);
    expect(alertBanner()).toBeNull();
    expect(minutesInput().value).toBe('20');
    const usdFloor = root.querySelector('[data-floor="USD"] input') as HTMLInputElement | null;
    expect(usdFloor?.value).toBe('1500');
  });

  it('row 2: load fails, then the shell Refresh, load fails again -> error shown (new text), form still empty', async () => {
    viewQueue.push(
      {
        status: 400,
        body: { error: { code: 'validationFailed', message: 'ข้อผิดพลาดครั้งที่ 1' } },
      },
      {
        status: 400,
        body: { error: { code: 'validationFailed', message: 'ข้อผิดพลาดครั้งที่ 2 หลังกดรีเฟรช' } },
      },
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);

    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();

    expect(viewCalls).toEqual([B01]);
    expect(alertBanner()?.textContent).toContain('ข้อผิดพลาดครั้งที่ 1');
    expect(minutesInput().value).toBe('');

    requestRefresh();
    await settle();

    expect(viewCalls).toEqual([B01, B01]);
    expect(alertBanner()?.textContent).toContain('ข้อผิดพลาดครั้งที่ 2 หลังกดรีเฟรช');
    expect(minutesInput().value).toBe('');
    expect(root.querySelector('[data-floor="USD"]')).toBeNull();
  });

  it('edge case (§9): a refresh tick while not shown does no read, and re-reads on refresh when shown', async () => {
    viewQueue.push(
      {
        status: 503,
        body: { error: { code: 'serviceUnavailable', message: 'unavailable' } },
      },
      {
        status: 200,
        body: { version: 3, changed: true, data: structuredClone(sampleSettings) },
      },
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown, setShown] = createSignal(true);

    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();

    expect(viewCalls).toEqual([B01]);
    expect(alertBanner()?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');

    setShown(false);
    await settle();

    requestRefresh();
    await settle();
    expect(viewCalls).toEqual([B01]);

    setShown(true);
    await settle();
    requestRefresh();
    await settle();

    expect(viewCalls.length).toBeGreaterThanOrEqual(2);
    expect(alertBanner()).toBeNull();
    expect(minutesInput().value).toBe('20');
  });

  it('read-order (a): a refresh read that answers after a successful Save leaves the saved value and version in place', async () => {
    let resolveInFlightRefresh!: (reply: { status: number; body: unknown }) => void;
    const inFlightRefresh = new Promise<{ status: number; body: unknown }>((r) => {
      resolveInFlightRefresh = r;
    });

    viewQueue.push(
      {
        status: 200,
        body: { version: 3, changed: true, data: structuredClone(sampleSettings) },
      },
      () => inFlightRefresh,
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);

    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();

    expect(minutesInput().value).toBe('20');

    // Start a refresh read that stays in flight while the user saves.
    requestRefresh();
    await settle();
    expect(viewCalls).toEqual([B01, B01]);

    // Save completes with new values and version 4.
    const savedSettings = {
      branch: B01,
      staleDataMinutes: 45,
      floors: [{ currency: 'USD', minQuantity: '2500' }],
      version: 4,
    };
    saveQueue.push({ status: 200, body: structuredClone(savedSettings) });
    saveButton().click();
    await settle();

    expect(minutesInput().value).toBe('45');
    const usdAfterSave = root.querySelector('[data-floor="USD"] input') as HTMLInputElement | null;
    expect(usdAfterSave?.value).toBe('2500');

    // Now the older refresh read answers with stale version 3 data.
    resolveInFlightRefresh({
      status: 200,
      body: { version: 3, changed: true, data: structuredClone(sampleSettings) },
    });
    await settle();

    // Saved value and version 4 remain in place.
    expect(minutesInput().value).toBe('45');
    const usdAfterStaleRead = root.querySelector('[data-floor="USD"] input') as HTMLInputElement | null;
    expect(usdAfterStaleRead?.value).toBe('2500');

    saveQueue.push({
      status: 200,
      body: { ...savedSettings, version: 5 },
    });
    saveButton().click();
    await settle();
    expect(saveBodies).toHaveLength(2);
    expect(saveBodies[1].expectedVersion).toBe(4);
  });

  it('read-order (b): of two overlapping refresh reads, an older 503 answering last leaves the newer data and shows no error', async () => {
    let resolveOlderRefresh!: (reply: { status: number; body: unknown }) => void;
    const olderRefresh = new Promise<{ status: number; body: unknown }>((r) => {
      resolveOlderRefresh = r;
    });

    const newerSettings = {
      branch: B01,
      staleDataMinutes: 35,
      floors: [{ currency: 'EUR', minQuantity: '900' }],
      version: 5,
    };

    viewQueue.push(
      {
        status: 200,
        body: { version: 3, changed: true, data: structuredClone(sampleSettings) },
      },
      () => olderRefresh,
      {
        status: 200,
        body: { version: 5, changed: true, data: structuredClone(newerSettings) },
      },
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);

    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();

    // First refresh starts and hangs.
    requestRefresh();
    await settle();

    // Second refresh starts and finishes first with newer data.
    requestRefresh();
    await settle();

    expect(viewCalls).toEqual([B01, B01, B01]);
    expect(minutesInput().value).toBe('35');
    const eurFloor = root.querySelector('[data-floor="EUR"] input') as HTMLInputElement | null;
    expect(eurFloor?.value).toBe('900');
    expect(alertBanner()).toBeNull();

    // Older refresh now fails with 503 last.
    resolveOlderRefresh({
      status: 503,
      body: { error: { code: 'serviceUnavailable', message: 'unavailable' } },
    });
    await settle();

    // Newer data stays in place and no error is shown.
    expect(alertBanner()).toBeNull();
    expect(minutesInput().value).toBe('35');
    const eurFloorAfter = root.querySelector('[data-floor="EUR"] input') as HTMLInputElement | null;
    expect(eurFloorAfter?.value).toBe('900');
  });
});
