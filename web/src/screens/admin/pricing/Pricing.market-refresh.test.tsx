// #5198 (PV-g): the TT and SR refresh buttons go through `POST /api/pricing/market-refresh` `{}` (keyed write), then
// read the board; a source answered `ok: false` shows `t('marketRefreshFailed', {source})` (TH "ดึงราคา {source} ไม่สำเร็จ")
// in the `.pricing-error` line; BoT keeps `POST /api/rate-references/fetch`. Up-front tests written before the
// implementation by claude/8be3a451 (XC FE lane controller). Do not edit, delete or weaken these tests; add your own in
// another file.
//
// Rows P9-P11 of the #5198 behaviour table (the rest are in Pricing.poll.test.tsx).
// `MarketSources` is replaced by a probe with one `button[name=refresh-<SOURCE>]` per source (TT, SR and BOT) that
// calls `onRefresh(source)`, because the real chip shows no BoT button; `PricingGrid` is replaced by an empty probe.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { For } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  makeMarketRatesRefreshOutput,
  makeMarketRefreshSourceDto,
} from '../../../shared/testing/fixtures/market_rates_refresh';
import { makeRateReferenceFetchOutput } from '../../../shared/testing/fixtures/rate_reference_fetch';
import { BOARD, DEFAULT_ME } from './__fixtures__/pricing-board.fixture';

vi.mock('./MarketSources', () => ({
  MarketSources: (p: { onRefresh(source: 'TT' | 'SR' | 'BOT'): void }) => (
    <div data-probe="sources">
      <For each={['TT', 'SR', 'BOT'] as const}>
        {(s) => <button type="button" name={`refresh-${s}`} onClick={() => p.onRefresh(s)} />}
      </For>
    </div>
  ),
}));

vi.mock('./PricingGrid', () => ({
  PricingGrid: () => <ul data-probe="grid" />,
}));

import { Pricing } from './Pricing';

const MARKET_REFRESH = '/api/pricing/market-refresh';
const RATE_REFERENCE_FETCH = '/api/rate-references/fetch';
const BOARD_VIEW = '/api/v3/views/pricing_board';

type Call = { method: string; path: string; key: string | null; body: unknown };

/** A fetch that answers `me`, `pricing_board` (always `changed: true` with `BOARD`) and the two refresh routes, and
 *  records every call in order. */
function fakeServer(marketRefresh: unknown = makeMarketRatesRefreshOutput()) {
  const calls: Call[] = [];
  let n = 0;
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = new URL(String(input), 'http://localhost').pathname;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, key: new Headers(init?.headers).get('idempotency-key'), body });
    let status = 200;
    let answer: unknown;
    if (method === 'GET' && path === BOARD_VIEW) answer = { version: ++n, changed: true, data: BOARD };
    else if (method === 'GET' && path === '/api/v3/views/me') answer = { version: 1, changed: true, data: DEFAULT_ME };
    else if (method === 'POST' && path === MARKET_REFRESH) answer = marketRefresh;
    else if (method === 'POST' && path === RATE_REFERENCE_FETCH) answer = makeRateReferenceFetchOutput();
    else {
      status = 404;
      answer = { error: { code: 'notFound', message: 'nf' } };
    }
    return new Response(JSON.stringify(answer), { status, headers: { 'content-type': 'application/json' } });
  };
  return { fn, calls };
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

async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await vi.advanceTimersByTimeAsync(0);
}

async function mount(server = fakeServer()) {
  vi.stubGlobal('fetch', vi.fn(server.fn));
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <Pricing branch={() => 'MAIN'} period={() => ({ from: '', to: '' })} shown={() => true} />,
    host,
  );
  await settle();
  return server;
}

function click(sel: string): void {
  const el = host.querySelector<HTMLButtonElement>(sel);
  expect(el, `${sel} exists`).not.toBeNull();
  el!.click();
}

const posts = (calls: Call[], path: string) => calls.filter((c) => c.method === 'POST' && c.path === path);
const boardReads = (calls: Call[]) => calls.filter((c) => c.method === 'GET' && c.path === BOARD_VIEW);

describe('Pricing market refresh (#5198, PV-g)', () => {
  it('P9 refresh on TT sends one keyed POST market-refresh {} and then reads the board; no rate-references/fetch', async () => {
    const server = await mount(
      fakeServer(makeMarketRatesRefreshOutput({ sources: [makeMarketRefreshSourceDto({ source: 'TT', ok: true })] })),
    );
    const readsBefore = boardReads(server.calls).length;
    click('button[name=refresh-TT]');
    await settle();

    const refresh = posts(server.calls, MARKET_REFRESH);
    expect(refresh).toHaveLength(1);
    expect(refresh[0]!.body).toEqual({});
    expect(refresh[0]!.key, 'Idempotency-Key header').toBeTruthy();
    expect(posts(server.calls, RATE_REFERENCE_FETCH)).toHaveLength(0);

    const postAt = server.calls.indexOf(refresh[0]!);
    const readAfter = server.calls.slice(postAt + 1).filter((c) => c.method === 'GET' && c.path === BOARD_VIEW);
    expect(readAfter.length, 'a board read after the POST').toBeGreaterThanOrEqual(1);
    expect(boardReads(server.calls).length).toBeGreaterThan(readsBefore);
  });

  it('P10 a source answered ok: false shows "ดึงราคา SR ไม่สำเร็จ" in the error line', async () => {
    await mount(
      fakeServer(
        makeMarketRatesRefreshOutput({
          sources: [
            makeMarketRefreshSourceDto({ source: 'TT', ok: true }),
            makeMarketRefreshSourceDto({ source: 'SR', ok: false, error: 'x' }),
          ],
        }),
      ),
    );
    click('button[name=refresh-SR]');
    await settle();
    const line = host.querySelector('.pricing-error');
    expect(line, '.pricing-error is shown').not.toBeNull();
    expect(line!.textContent?.trim()).toBe('ดึงราคา SR ไม่สำเร็จ');
  });

  it('P11 refresh on BoT sends POST rate-references/fetch as today', async () => {
    const server = await mount();
    click('button[name=refresh-BOT]');
    await settle();
    const fetches = posts(server.calls, RATE_REFERENCE_FETCH);
    expect(fetches).toHaveLength(1);
    expect(fetches[0]!.key, 'Idempotency-Key header').toBeTruthy();
    expect(posts(server.calls, MARKET_REFRESH)).toHaveLength(0);
  });
});
