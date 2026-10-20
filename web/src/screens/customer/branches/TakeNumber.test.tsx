// C08 up-front tests (feat/fe-customer-2 skeleton): TakeNumber takes a queue number from the app with
// `queueTicketTake` `POST /api/app/queue/tickets` `{branch}` and an Idempotency-Key (BE contract #3410; it answers
// `MyQueueTicketData`, refuses 409 `alreadyQueued` / `queueClosed`, 401 with no session). The implementer makes
// these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { Fetch } from '../../../shared/api/client';
import { makeMyQueueTicketData } from '../../../shared/testing/fixtures/my_queue_ticket';
import { setLang } from '../../../shared/i18n';
import { TakeNumber } from './TakeNumber';

type Reply = { status: number; body: unknown } | 'network';

const TICKET = makeMyQueueTicketData({
  ticketId: 't-9', branch: 'B01', displayNumber: 'A012', serviceType: 'WALK_IN', status: 'WAITING',
  ticketsAhead: 4, estimateSeconds: 600, estimateLowSeconds: 420, estimateHighSeconds: 780,
  estimateAsOf: '2026-10-19T08:00:00Z',
});

function server(replies: Reply[] = []) {
  const writes: { path: string; key: string | null; body: unknown }[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const path = new URL(input, 'http://localhost').pathname;
    if ((init?.method ?? 'GET').toUpperCase() !== 'POST' || path !== '/api/app/queue/tickets') {
      return new Response('{}', { status: 404 });
    }
    writes.push({ path, key: new Headers(init?.headers).get('idempotency-key'), body: JSON.parse(String(init?.body ?? '{}')) });
    const r = replies.shift() ?? { status: 200, body: TICKET };
    if (r === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } });
  });
  return { fetchFn, writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(fetchFn: Fetch, queueOpen = true): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <TakeNumber branch="B01" queueOpen={queueOpen} fetch={fetchFn} />, host);
}
const button = () => host.querySelector('button[data-action="take-number"]') as HTMLButtonElement | null;
const link = () => host.querySelector('a[data-testid="take-number-link"]') as HTMLAnchorElement | null;
const message = () => host.querySelector('[data-testid="take-number-message"]');

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('TakeNumber (C08)', () => {
  it('a closed queue offers no button', () => {
    mount(server().fetchFn, false);
    expect(button()).toBeNull();
  });

  it('takes a number: one keyed POST {branch}, then the number and a link to that ticket on the journey', async () => {
    const s = server();
    mount(s.fetchFn);
    button()!.click();
    await flush();
    expect(s.writes).toEqual([{ path: '/api/app/queue/tickets', key: expect.any(String), body: { branch: 'B01' } }]);
    expect(s.writes[0].key).toBeTruthy();
    expect(host.textContent).toContain('A012');
    expect(link()?.getAttribute('href')).toBe('/app/journey?branch=B01&ticketId=t-9');
    expect(button()).toBeNull();
  });

  it('the button is disabled while the take is in flight, so one tap sends one write', async () => {
    const s = server();
    mount(s.fetchFn);
    button()!.click();
    expect(button()!.disabled).toBe(true);
    button()!.click();
    await flush();
    expect(s.writes).toHaveLength(1);
  });

  it('409 alreadyQueued says so and links to the journey for this branch, where the ticket is found', async () => {
    mount(server([{ status: 409, body: { error: { code: 'alreadyQueued', message: 'already' } } }]).fetchFn);
    button()!.click();
    await flush();
    expect(message()?.textContent?.trim()).toBeTruthy();
    expect(link()?.getAttribute('href')).toBe('/app/journey?branch=B01');
  });

  it('409 queueClosed says the branch is not taking numbers, with no link', async () => {
    mount(server([{ status: 409, body: { error: { code: 'queueClosed', message: 'closed' } } }]).fetchFn);
    button()!.click();
    await flush();
    expect(message()?.textContent?.trim()).toBeTruthy();
    expect(link()).toBeNull();
  });

  it('401 offers the sign-in link', async () => {
    mount(server([{ status: 401, body: { error: { code: 'signIn', message: 'sign in' } } }]).fetchFn);
    button()!.click();
    await flush();
    expect(host.querySelector('a[href="/app/sign-in"]')).not.toBeNull();
  });

  it('a network failure shows the error and the retry reuses the key', async () => {
    const s = server(['network']);
    mount(s.fetchFn);
    button()!.click();
    await flush();
    expect(message()?.textContent?.trim()).toBeTruthy();
    button()!.click();
    await flush();
    expect(s.writes).toHaveLength(2);
    expect(s.writes[1].key).toBe(s.writes[0].key);
    expect(link()?.getAttribute('href')).toBe('/app/journey?branch=B01&ticketId=t-9');
  });
});
