import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import type { AdminMenuProps } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Rates } from './Rates';
import { th as ratesTh } from './i18n';

const REFERENCE = '/api/v3/views/rates_reference';

interface RefRow {
  code: string;
  date: string;
  unit: number;
  buyingSight: string;
  buyingTransfer: string;
  selling: string;
}

interface RefData {
  rows: RefRow[];
  fetchedAt: string;
  error: string | null;
}

type RefAnswer =
  | 'network'
  | { status: number; body: unknown };

function sampleRefData(over: Partial<RefData> = {}): RefData {
  return {
    rows: [
      {
        code: 'USD',
        date: '2026-10-06',
        unit: 1,
        buyingSight: '33.4218',
        buyingTransfer: '33.5008',
        selling: '33.8278',
      },
    ],
    fetchedAt: '2026-10-06T04:30:00.000Z',
    error: null,
    ...over,
  };
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    if (vi.isFakeTimers()) {
      await vi.advanceTimersByTimeAsync(1);
    } else {
      await new Promise((r) => setTimeout(r, 0));
    }
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe('Admin Rates reference poll recovery (#2159)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let refQueue: RefAnswer[];
  let refCalls: { since: string | null; fresh: string | null }[];

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    setLang('th');
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    refQueue = [];
    refCalls = [];

    const fetchMock = vi.fn(async (input: RequestInfo | URL): Promise<Response> => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname === '/api/v3/views/me') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: {
            kind: 'owner',
            id: 'owner',
            role: 'OWNER',
            branch: 'MAIN',
            name: 'Owner',
            username: 'owner',
            permissions: [],
          },
        });
      }
      if (url.pathname === QUEUE_CARD_PATH) {
        return jsonResponse(200, queueCardUnchanged());
      }
      if (url.pathname === '/api/v3/views/published_rates') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: { current: null, lines: [] },
        });
      }
      if (url.pathname === '/api/v3/views/rate_drafts') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: { drafts: [] },
        });
      }
      if (url.pathname === '/api/v3/views/rate_policy') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: { policy: null },
        });
      }
      if (url.pathname === REFERENCE) {
        refCalls.push({
          since: url.searchParams.get('since'),
          fresh: url.searchParams.get('fresh'),
        });
        const next = refQueue.shift() ?? {
          status: 200,
          body: { version: 3, changed: false },
        };
        if (next === 'network') {
          throw new TypeError('Failed to fetch');
        }
        return jsonResponse(next.status, next.body);
      }
      return jsonResponse(200, { version: 1, changed: false });
    });

    vi.stubGlobal('fetch', fetchMock);
    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    setLang('th');
    await settle();
  });

  function mountScreen(): void {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(
      () =>
        (
          <>
            <Rates {...props} />
            <Dialog />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
  }

  function botSection(): HTMLElement {
    const el = root.querySelector<HTMLElement>('.bot-reference-section');
    expect(el).not.toBeNull();
    return el!;
  }

  function botAlert(): HTMLElement | null {
    return botSection().querySelector<HTMLElement>('.bot-error[role="alert"]');
  }

  it('row 1: a poll fails, then the next poll answers changed: false, and the held data has no error -> the alert is gone', async () => {
    refQueue.push(
      {
        status: 200,
        body: { version: 5, changed: true, data: sampleRefData({ error: null }) },
      },
      {
        status: 503,
        body: { error: { code: 'serverError', message: 'poll failed' } },
      },
      {
        status: 200,
        body: { version: 5, changed: false },
      },
    );

    mountScreen();
    await settle();
    expect(refCalls.length).toBe(1);
    expect(botAlert()).toBeNull();
    expect(botSection().textContent).toContain('USD');

    // First 60s poll fails -> alert appears
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(refCalls.length).toBe(2);
    expect(refCalls[1]!.since).toBe('5');
    expect(botAlert()?.textContent).toBe(sharedTh['error.serverError']);

    // Next 60s poll answers changed: false -> alert is gone and held rows remain shown
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(refCalls.length).toBe(3);
    expect(refCalls[2]!.since).toBe('5');
    expect(botAlert()).toBeNull();
    expect(botSection().textContent).toContain('USD');
  });

  it('row 2: a poll fails, then changed: false, and the held data\'s own error is "source unavailable" -> that source error is shown, not the failure text', async () => {
    refQueue.push(
      {
        status: 200,
        body: { version: 7, changed: true, data: sampleRefData({ error: 'source unavailable' }) },
      },
      'network',
      {
        status: 200,
        body: { version: 7, changed: false },
      },
    );

    mountScreen();
    await settle();
    expect(botAlert()?.textContent).toBe('source unavailable');

    // Poll fails with network error -> failure text replaces it
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()?.textContent).toBe(sharedTh['error.network']);

    // Next poll answers changed: false -> reverts to the held data's own error ("source unavailable")
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()?.textContent).toBe('source unavailable');
    expect(botAlert()?.textContent).not.toBe(sharedTh['error.network']);
  });

  it('row 3: a poll fails, then changed: true with new data -> as today (the new data\'s error, or none)', async () => {
    refQueue.push(
      {
        status: 200,
        body: { version: 2, changed: true, data: sampleRefData({ error: 'initial source warning' }) },
      },
      'network',
      {
        status: 200,
        body: { version: 3, changed: true, data: sampleRefData({ error: null }) },
      },
      'network',
      {
        status: 200,
        body: { version: 4, changed: true, data: sampleRefData({ error: 'upstream rate feed delayed' }) },
      },
    );

    mountScreen();
    await settle();
    expect(botAlert()?.textContent).toBe('initial source warning');

    // Poll 1 fails
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()?.textContent).toBe(sharedTh['error.network']);

    // Poll 2 answers changed: true with error: null -> alert is gone
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()).toBeNull();

    // Poll 3 fails
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()?.textContent).toBe(sharedTh['error.network']);

    // Poll 4 answers changed: true with new data's error -> shows that error
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(botAlert()?.textContent).toBe('upstream rate feed delayed');
  });

  it('row 4: the first load fails, then the next poll answers changed: false (no data held) -> the alert is gone, and the panel shows its empty state as today', async () => {
    refQueue.push(
      {
        status: 500,
        body: { error: { code: 'serverError', message: 'initial load failed' } },
      },
      {
        status: 200,
        body: { version: 1, changed: false },
      },
    );

    mountScreen();
    await settle();
    expect(refCalls.length).toBe(1);
    expect(botAlert()?.textContent).toBe(sharedTh['error.serverError']);
    expect(botSection().querySelector('table.comparison-table')).toBeNull();

    // Next poll answers changed: false while refData() is still null -> alert is gone, empty state remains
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(refCalls.length).toBe(2);
    expect(botAlert()).toBeNull();
    expect(botSection().querySelector('.loading')).toBeNull();
    expect(botSection().querySelector('.fetched')).toBeNull();
    expect(botSection().querySelector('table.comparison-table')).toBeNull();
  });

  it('row 5: the "fresh" button fails -> the alert shows, as today', async () => {
    refQueue.push(
      {
        status: 200,
        body: { version: 3, changed: true, data: sampleRefData({ error: null }) },
      },
      {
        status: 502,
        body: { error: { code: 'serverError', message: 'BOT gateway timeout' } },
      },
    );

    mountScreen();
    await settle();
    expect(botAlert()).toBeNull();

    const freshBtn = [...botSection().querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => (b.textContent ?? '').trim() === ratesTh.botRefreshButton,
    );
    expect(freshBtn).toBeDefined();
    freshBtn!.click();
    await settle();

    expect(refCalls.length).toBe(2);
    expect(refCalls[1]!.fresh).toBe('1');
    expect(botAlert()?.textContent).toBe(sharedTh['error.serverError']);
  });
});
