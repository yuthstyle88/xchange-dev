// QW-4 (kiosk-shared-phone §5r F14, rows P1, P2): the signed-out calling display, numbers only (owner R-7).
// Written before the implementation by the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these
// tests; add tests in a file of your own.
//
// Pinned:
// - `web/src/screens/public/queue-display/QueueDisplay.tsx` exports
//     QueueDisplay(props: { fetch: (url: string, init?: RequestInit) => Promise<Response>; search: string;
//                           intervalMs?: number })
//   (the same injection as RateDisplay, so tests mock at the fetch level).
// - It reads `/api/public/host` for `{org}` (§4.3a W10), then polls
//   `/api/public/orgs/{org}/branches/{code}/queue-display` (W9) with the seam: the first read has no `since`, every
//   later read sends `since=<version>` of the last answer; `{changed: false}` keeps what is shown (P1).
// - It renders each called number in an element with [data-called] and each recent one with [data-recent].
// - It renders **only** numbers: a name, phone, amount, teller or counter that a server might wrongly add to the
//   JSON never reaches the page (P2).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueDisplay } from './QueueDisplay';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let urls: string[];
let answers: Array<() => Response>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const FIRST = {
  version: 7,
  changed: true,
  data: {
    called: [{ displayNumber: 'K042', status: 'CALLED', calledAt: '2026-10-21T10:00:00Z', name: 'Somchai', phone: '0812345678' }],
    recent: [
      { displayNumber: 'K041', calledAt: '2026-10-21T09:58:00Z', amount: '3210.00', teller: 'teller1', counter: 'C2' },
      { displayNumber: 'K040', calledAt: '2026-10-21T09:55:00Z' },
    ],
  },
};

const fetchFn = vi.fn(async (url: string) => {
  urls.push(url);
  if (url === '/api/public/host') return json(200, { org: 'ORG_1' });
  const next = answers.shift();
  return next ? next() : json(200, { version: 7, changed: false });
});

async function tick(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  urls = [];
  answers = [() => json(200, FIRST)];
  fetchFn.mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

function mount(): void {
  dispose = render(
    () => <QueueDisplay fetch={fetchFn} search="?branch=MAIN" intervalMs={5_000} />,
    host,
  );
}

const displayUrls = () => urls.filter((u) => u.includes('/queue-display'));

describe('QW-4: the calling display', () => {
  it('reads the host, then the branch display; shows the called and the recent numbers', async () => {
    mount();
    await tick(0);
    expect(urls[0]).toBe('/api/public/host');
    expect(displayUrls()[0]).toMatch(/^\/api\/public\/orgs\/ORG_1\/branches\/MAIN\/queue-display(\?|$)/);
    expect(displayUrls()[0]).not.toMatch(/since=/);
    expect([...host.querySelectorAll('[data-called]')].map((e) => e.textContent?.trim())).toEqual(['K042']);
    expect([...host.querySelectorAll('[data-recent]')].map((e) => e.textContent?.trim())).toEqual(['K041', 'K040']);
  });

  it('P1: the next poll sends since=<version>; {changed:false} keeps the same numbers on screen', async () => {
    mount();
    await tick(0);
    const before = host.querySelector('[data-called]');
    await tick(5_000);
    expect(displayUrls().length).toBeGreaterThanOrEqual(2);
    expect(displayUrls()[1]).toMatch(/[?&]since=7(&|$)/);
    expect(host.querySelector('[data-called]')?.textContent?.trim()).toBe('K042');
    expect(host.querySelector('[data-called]'), 'the same node, not a re-render').toBe(before);
  });

  it('a new call replaces the called number', async () => {
    answers.push(() =>
      json(200, { version: 8, changed: true, data: { called: [{ displayNumber: 'K043', status: 'CALLED', calledAt: '2026-10-21T10:02:00Z' }], recent: [] } }),
    );
    mount();
    await tick(0);
    await tick(5_000);
    expect([...host.querySelectorAll('[data-called]')].map((e) => e.textContent?.trim())).toEqual(['K043']);
  });

  it('P2: numbers only: no name, phone, amount, teller or counter from the JSON is rendered', async () => {
    mount();
    await tick(0);
    const text = host.textContent ?? '';
    for (const leaked of ['Somchai', '0812345678', '3210.00', 'teller1', 'C2']) {
      expect(text, `must not render "${leaked}"`).not.toContain(leaked);
    }
  });
});
