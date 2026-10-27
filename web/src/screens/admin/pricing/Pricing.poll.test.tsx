// #5198 (V.9 B6): `/pricing` polls `pricing_board` every 60 s through `ViewPoller`, pauses while a currency is open or
// a row is selected, and stops on branch change and unmount. Up-front tests written before the implementation by
// claude/8be3a451 (XC FE lane controller). Do not edit, delete or weaken these tests; add your own in another file.
//
// Rows P1-P8 and P12 of the #5198 behaviour table (P9-P11 are in Pricing.market-refresh.test.tsx).
// `PricingGrid` is replaced by a probe that lists the rows and exposes `onOpen` / `onToggle`, so this file depends on
// no child's markup. The detail panel's close button is `button[name="close-detail"]` (text `t('closeDetail')`, TH "ปิด").
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { For, createSignal } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { BOARD, DEFAULT_ME } from './__fixtures__/pricing-board.fixture';

vi.mock('./PricingGrid', () => ({
  PricingGrid: (p: {
    rows: readonly { currency: string }[];
    onOpen(currency: string): void;
    onToggle(currency: string): void;
  }) => (
    <ul data-probe="grid">
      <For each={p.rows}>
        {(r) => (
          <li data-currency={r.currency}>
            <button type="button" data-open={r.currency} onClick={() => p.onOpen(r.currency)} />
            <button type="button" data-toggle={r.currency} onClick={() => p.onToggle(r.currency)} />
          </li>
        )}
      </For>
    </ul>
  ),
}));

import { Pricing } from './Pricing';

type Read = { branch: string | null; since: string | null };
type BoardAnswer = { version: number; changed: boolean; data?: unknown };

/** A fetch that answers `me` and `pricing_board` (envelope `{version, changed, data}`) and records every board read
 *  with its `branch` and `since` query values. `answer(n)` gives the n-th board answer (1-based); by default every
 *  read answers `changed: true` with `BOARD` at version `100 + n`. */
function fakeServer(answer: (n: number) => BoardAnswer = (n) => ({ version: 100 + n, changed: true, data: BOARD })) {
  const reads: Read[] = [];
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const url = new URL(String(input), 'http://localhost');
    let body: unknown;
    let status = 200;
    if (method === 'GET' && url.pathname === '/api/v3/views/pricing_board') {
      reads.push({ branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      body = answer(reads.length);
    } else if (method === 'GET' && url.pathname === '/api/v3/views/me') {
      body = { version: 1, changed: true, data: DEFAULT_ME };
    } else if (method === 'POST') {
      body = {};
    } else {
      status = 404;
      body = { error: { code: 'notFound', message: 'nf' } };
    }
    return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  };
  return { fn, reads };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

/** Let pending fetches, promise chains and zero-delay timers run, without moving the clock forward. */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await vi.advanceTimersByTimeAsync(0);
}

async function advance(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
  await settle();
}

async function mount(server = fakeServer(), initialBranch = 'MAIN') {
  vi.stubGlobal('fetch', vi.fn(server.fn));
  const [branch, setBranch] = createSignal(initialBranch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <Pricing branch={branch} period={() => ({ from: '', to: '' })} shown={() => true} />,
    host,
  );
  await settle();
  return { server, setBranch };
}

const shown = () => [...host.querySelectorAll('[data-probe=grid] li')].map((li) => li.getAttribute('data-currency'));

function click(sel: string): void {
  const el = host.querySelector<HTMLButtonElement>(sel);
  expect(el, `${sel} exists`).not.toBeNull();
  el!.click();
}

describe('Pricing polls pricing_board (#5198)', () => {
  it('P1 mount at branch MAIN reads pricing_board once with branch=MAIN', async () => {
    const { server } = await mount();
    expect(server.reads).toHaveLength(1);
    expect(server.reads[0]!.branch).toBe('MAIN');
  });

  it('P2 after 60 s a second read sends the version from the first answer', async () => {
    const { server } = await mount();
    expect(server.reads).toHaveLength(1);
    await advance(60_000);
    expect(server.reads).toHaveLength(2);
    expect(server.reads[1]!.branch).toBe('MAIN');
    expect(server.reads[1]!.since).toBe('101');
  });

  it('P3 a second answer {changed: false} keeps the board as it was', async () => {
    const { server } = await mount(
      fakeServer((n) => (n === 1 ? { version: 101, changed: true, data: BOARD } : { version: 101, changed: false })),
    );
    const before = shown();
    expect(before).toEqual(BOARD.rows.map((r) => r.currency));
    await advance(60_000);
    expect(server.reads, 'the 60 s poll read happened').toHaveLength(2);
    expect(shown()).toEqual(before);
  });

  it('P4 no read while a currency is open', async () => {
    const { server } = await mount();
    await advance(60_000);
    expect(server.reads, 'polling runs before the panel opens').toHaveLength(2);
    click('[data-open=USD]');
    await settle();
    const n = server.reads.length;
    await advance(120_000);
    expect(server.reads).toHaveLength(n);
  });

  it('P5 pressing close-detail resumes polling: a read within 60 s', async () => {
    const { server } = await mount();
    click('[data-open=USD]');
    await settle();
    await advance(60_000);
    const n = server.reads.length;
    const close = host.querySelector<HTMLButtonElement>('button[name="close-detail"]');
    expect(close, 'button[name="close-detail"] exists').not.toBeNull();
    expect(close!.textContent?.trim()).toBe('ปิด');
    close!.click();
    await settle();
    await advance(60_000);
    expect(server.reads.length).toBeGreaterThan(n);
  });

  it('P6 no read while a row is selected; deselecting resumes polling', async () => {
    const { server } = await mount();
    click('[data-toggle=USD]');
    await settle();
    const n = server.reads.length;
    await advance(120_000);
    expect(server.reads, 'no read while USD is selected').toHaveLength(n);
    click('[data-toggle=USD]');
    await settle();
    await advance(60_000);
    expect(server.reads.length, 'a read after deselecting').toBeGreaterThan(n);
  });

  it('P7 reload while a currency is open reads at once', async () => {
    const { server } = await mount();
    click('[data-open=USD]');
    await settle();
    const n = server.reads.length;
    click('button[name=reload]');
    await settle();
    expect(server.reads).toHaveLength(n + 1);
  });

  it('P8 switching the branch to B2: the next read uses B2 and no later read uses MAIN', async () => {
    const { server, setBranch } = await mount();
    const n = server.reads.length;
    setBranch('B2');
    await settle();
    await advance(60_000);
    await advance(120_000);
    const after = server.reads.slice(n);
    expect(after.length, 'a read right after the switch and a 60 s poll on B2').toBeGreaterThanOrEqual(2);
    expect(after[0]!.branch).toBe('B2');
    expect(after.map((r) => r.branch).filter((b) => b !== 'B2')).toEqual([]);
  });

  it('P12 no read after unmount', async () => {
    const { server } = await mount();
    await advance(60_000);
    expect(server.reads, 'polling runs while mounted').toHaveLength(2);
    dispose!();
    dispose = undefined;
    await advance(300_000);
    expect(server.reads).toHaveLength(2);
  });
});
