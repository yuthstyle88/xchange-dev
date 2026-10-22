// PH-queue-display up-front tests (docs/design/web-public-get.md §4, rows Q1–Q5). Written before the implementation
// by the XC FE controller's skeleton session (claude/e8020026). Do not edit, delete or weaken these tests; add tests
// in a file of your own. The harness is copied from QueueDisplay.test.tsx (that file is not edited).
//
// Pinned: the signed-out calling display is a wall screen nobody reloads. It reads the host org through
// `readHostOrg`; when that read fails (a 5xx, a network failure, or an answer with no org), it does not give up:
// every tick of its interval re-reads the host until one succeeds, then makes its first display read (no `since`)
// in that same tick. Once the org is known, the host is not read again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueDisplay } from './QueueDisplay';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let urls: string[];
let hostAnswers: Array<() => Response>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const FIRST = {
  version: 7,
  changed: true,
  data: {
    called: [{ displayNumber: 'K042', status: 'CALLED', calledAt: '2026-10-21T10:00:00Z' }],
    recent: [{ displayNumber: 'K041', calledAt: '2026-10-21T09:58:00Z' }],
  },
};

let displayAnswers: Array<() => Response>;

const fetchFn = vi.fn(async (url: string) => {
  urls.push(url);
  if (url === '/api/public/host') {
    const next = hostAnswers.shift();
    return next ? next() : json(200, { org: 'ORG_1' });
  }
  const next = displayAnswers.shift();
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
  hostAnswers = [];
  displayAnswers = [() => json(200, FIRST)];
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

const hostUrls = () => urls.filter((u) => u === '/api/public/host');
const displayUrls = () => urls.filter((u) => u.includes('/queue-display'));
const calledNumbers = () => [...host.querySelectorAll('[data-called]')].map((e) => e.textContent?.trim());

describe('QueueDisplay: a failed host read is retried on the next tick (PH-queue-display)', () => {
  it.each<[string, () => Response]>([
    ['a 500', () => json(500, { error: { code: 'serverError', message: 'boom' } })],
    ['a 502 with a text body', () => new Response('Bad Gateway', { status: 502 })],
    ['an answer with no org', () => json(200, {})],
    [
      'a network failure',
      () => {
        throw new TypeError('Failed to fetch');
      },
    ],
  ])('Q1: after %s on load, the next tick re-reads the host and shows the numbers', async (_why, failure) => {
    hostAnswers = [failure];
    mount();
    await tick(0);
    expect(hostUrls()).toHaveLength(1);
    expect(displayUrls(), 'no display read without an org').toHaveLength(0);
    expect(calledNumbers()).toEqual([]);

    await tick(5_000);
    expect(hostUrls(), 'the host is read again on the next tick').toHaveLength(2);
    expect(displayUrls()).toHaveLength(1);
    expect(displayUrls()[0]).toMatch(/^\/api\/public\/orgs\/ORG_1\/branches\/MAIN\/queue-display$/);
    expect(calledNumbers()).toEqual(['K042']);
  });

  it('Q2: while the host keeps failing, every tick tries again and nothing else is read', async () => {
    const fail = () => json(503, { error: { code: 'serverError', message: 'down' } });
    hostAnswers = [fail, fail, fail];
    mount();
    await tick(0);
    await tick(5_000);
    await tick(5_000);
    expect(hostUrls()).toHaveLength(3);
    expect(displayUrls()).toHaveLength(0);

    await tick(5_000);
    expect(hostUrls()).toHaveLength(4);
    expect(calledNumbers()).toEqual(['K042']);
  });

  it('Q3: once the org is known the host is not read again; later ticks send since=<version>', async () => {
    hostAnswers = [() => json(500, { error: { code: 'serverError', message: 'boom' } })];
    mount();
    await tick(0);
    await tick(5_000);
    await tick(5_000);
    await tick(5_000);
    expect(hostUrls()).toHaveLength(2);
    expect(displayUrls().length).toBeGreaterThanOrEqual(3);
    expect(displayUrls()[0]).not.toMatch(/since=/);
    expect(displayUrls()[1]).toMatch(/[?&]since=7(&|$)/);
  });

  it('Q4: a good host on load is read once and the display read follows at once (unchanged)', async () => {
    mount();
    await tick(0);
    expect(hostUrls()).toHaveLength(1);
    expect(displayUrls()).toHaveLength(1);
    expect(calledNumbers()).toEqual(['K042']);
    await tick(5_000);
    expect(hostUrls()).toHaveLength(1);
  });
});
