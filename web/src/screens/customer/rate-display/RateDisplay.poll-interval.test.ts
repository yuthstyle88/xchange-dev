import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createComponent } from 'solid-js';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { makePublishedPublicationDto, makePublishedRatesData } from '../../../shared/testing/fixtures';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { HIDE_AFTER_MS } from './board/board-status';
import { RateDisplay, RATES_POLL_MS, type RateDisplayProps } from './RateDisplay';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

let fs: NodeFs;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  fs = (await import(/* @vite-ignore */ fsName)) as NodeFs;
  dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-24T00:00:00Z' }),
  sourceDate: '2026-10-24',
  stale: false,
  bandHeadings: [],
  lines: [],
});

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'br-1',
      code: 'MAIN',
      name: 'Main Branch',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

function createServer() {
  const calls: { url: string; path: string }[] = [];
  const fetch = vi.fn(async (url: string): Promise<Response> => {
    const path = new URL(url, 'http://board.test').pathname;
    calls.push({ url, path });

    if (path === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (path === '/api/public/orgs/XC/branches') {
      return new Response(JSON.stringify(BRANCHES), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (path === '/api/public/orgs/XC/branches/MAIN/rates') {
      return new Response(JSON.stringify(RATES), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response('{}', { status: 404 });
  });

  return {
    fetch,
    calls,
    ratesCalls: () => calls.filter((c) => c.path === '/api/public/orgs/XC/branches/MAIN/rates'),
  };
}

let dispose: (() => void) | undefined;

function mount(props: RateDisplayProps) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => createComponent(RateDisplay, props), host);
  return host;
}

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 10; i += 1) {
    await vi.advanceTimersByTimeAsync(20);
  }
}

describe('RateDisplay poll interval and hide threshold (#5057)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    window.localStorage.clear();
    setAudience('customer');
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.useRealTimers();
    setLang('th');
    setAudience('staff');
  });

  it('main.tsx stops passing ratesIntervalMs so the board uses RATES_POLL_MS', () => {
    const mainPath = `${dir}main.tsx`;
    expect(fs.existsSync(mainPath), 'main.tsx exists').toBe(true);
    const content = fs.readFileSync(mainPath, 'utf8');
    expect(content).not.toMatch(/ratesIntervalMs/);
  });

  it('RATES_POLL_MS is 10 s and strictly less than HIDE_AFTER_MS (30 s)', () => {
    expect(RATES_POLL_MS).toBe(10_000);
    expect(HIDE_AFTER_MS).toBe(30_000);
    expect(RATES_POLL_MS).toBeLessThan(HIDE_AFTER_MS);
  });

  it('defaults to RATES_POLL_MS when ratesIntervalMs is omitted', async () => {
    const s = createServer();
    mount({ fetch: s.fetch, search: '?branch=MAIN' });
    await settle();

    expect(s.ratesCalls()).toHaveLength(1);

    // After 9 s, still only initial call
    await settle(9_000);
    expect(s.ratesCalls()).toHaveLength(1);

    // Reaching RATES_POLL_MS (10 s total) triggers the second poll
    await settle(1_000);
    expect(s.ratesCalls()).toHaveLength(2);
  });

  it('a test passing ratesIntervalMs still uses the value it passes', async () => {
    const s = createServer();
    mount({ fetch: s.fetch, search: '?branch=MAIN', ratesIntervalMs: 5_000 });
    await settle();

    expect(s.ratesCalls()).toHaveLength(1);

    // After 4 s, still only initial call
    await settle(4_000);
    expect(s.ratesCalls()).toHaveLength(1);

    // Reaching 5 s triggers the second poll
    await settle(1_000);
    expect(s.ratesCalls()).toHaveLength(2);
  });
});
