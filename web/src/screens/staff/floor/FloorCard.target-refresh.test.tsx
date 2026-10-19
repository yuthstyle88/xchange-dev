// #2635: the cash-target inputs must follow a newer server target after a
// floor re-read, keeping only what the user typed and has not saved yet.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeStaffFloorTarget,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffFloorView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

const BRANCH = 'MAIN';

function me() {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000f1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions: ['AttendanceClock', 'CashTargetSet'],
  });
}

function floor(amounts: Record<string, string> | null) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: amounts ? makeStaffFloorTarget({ amounts, at: '2026-10-11T01:00:00Z' }) : null,
    safe: null,
  });
}

function line(currency: string) {
  return makePublishedLineDto({ currency, group: 'ALL', unitSize: '1', buyRate: '1.0000', sellRate: '1.1000', lineId: `l-${currency}` });
}

// Rates keep USD and EUR inputs on screen even when the target drops a currency.
const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: '11111111-2222-4333-8444-555555555555', version: 7, publishedAt: '2026-10-11T02:30:00Z' }),
  sourceDate: '2026-10-10',
  stale: false,
  lines: [line('USD'), line('EUR')],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function typeInto(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

interface Sent {
  path: string;
  body: string | undefined;
}

describe('FloorCard target inputs follow a re-read (#2635)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: Sent[];
  let targetAnswer: () => Response;

  beforeEach(() => {
    setLang('th');
    posts = [];
    targetAnswer = () => json(200, {});
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return json(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST') {
          posts.push({ path, body: init?.body as string | undefined });
          if (path === '/api/branch-day/target') return targetAnswer();
          return json(200, {});
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  /** Mounts the card the way StaffPage does: a fresh ctx object on every floor read. */
  async function mount(first: Record<string, string> | null, next?: Record<string, string> | null) {
    const [fl, setFl] = createSignal<StaffFloorView>(floor(first));
    const refresh = vi.fn(async () => {
      if (next !== undefined) setFl(floor(next));
    });
    const ctx = (): StaffCtx => ({ me: me(), floor: fl(), refresh });
    dispose = render(() => <FloorCard ctx={ctx()} />, host);
    await flush();
    return { reread: (a: Record<string, string> | null) => setFl(floor(a)), refresh };
  }

  const input = (cur: string): HTMLInputElement =>
    host.querySelector<HTMLInputElement>(`[data-currency="${cur}"]`)!;
  const save = (): HTMLButtonElement =>
    host.querySelector<HTMLButtonElement>('[data-testid="floor-target-save"]')!;

  it('row 1: USD 1000 shown, re-read answers USD 1200.50, nothing typed -> input shows 1200.50 (string kept as is)', async () => {
    const { reread } = await mount({ USD: '1000.00' });
    expect(input('USD').value).toBe('1000.00');

    reread({ USD: '1200.50' });
    await flush();
    expect(input('USD').value).toBe('1200.50');
  });

  it('row 2: EUR 600 typed (not saved), re-read answers EUR 500 / USD 1200 -> EUR keeps 600, USD shows 1200', async () => {
    const { reread } = await mount({ USD: '1000', EUR: '400' });
    typeInto(input('EUR'), '600');

    reread({ USD: '1200', EUR: '500' });
    await flush();
    expect(input('EUR').value).toBe('600');
    expect(input('USD').value).toBe('1200');
  });

  it("row 3: Save succeeds, re-read answers the normalised values -> inputs show the server's values", async () => {
    await mount({ USD: '1000', EUR: '400' }, { USD: '1200.5000', EUR: '400.0000' });
    typeInto(input('USD'), '1200.5');
    save().click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(JSON.parse(posts[0]!.body!).amounts).toEqual({ USD: '1200.5', EUR: '400' });
    expect(input('USD').value).toBe('1200.5000');
    expect(input('EUR').value).toBe('400.0000');
  });

  it('row 4 (kept): Save refused -> typed values kept, nothing re-read', async () => {
    targetAnswer = () => json(422, { error: { code: 'validationFailed', message: 'bad amount' } });
    const { refresh } = await mount({ USD: '1000' }, { USD: '9999' });
    typeInto(input('USD'), '12x');
    save().click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(refresh).not.toHaveBeenCalled();
    expect(input('USD').value).toBe('12x');
    expect(host.querySelector('[data-testid="floor-error"]')).not.toBeNull();
  });

  it('row 5: re-read answers no target for a currency that had one, nothing typed -> that input is empty', async () => {
    const { reread } = await mount({ USD: '1000', EUR: '400' });
    expect(input('EUR').value).toBe('400');

    reread({ USD: '1000' });
    await flush();
    expect(input('EUR').value).toBe('');
    expect(input('USD').value).toBe('1000');
  });

  it('row 5b: re-read answers no target at all, nothing typed -> the inputs are empty', async () => {
    const { reread } = await mount({ USD: '1000', EUR: '400' });

    reread(null);
    await flush();
    expect(input('USD').value).toBe('');
    expect(input('EUR').value).toBe('');
  });
});
