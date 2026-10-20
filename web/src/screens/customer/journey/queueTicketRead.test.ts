// C09 up-front tests (feat/fe-customer-2 skeleton): readMyQueueTicket finds the customer's own ticket with
// `my_queue_ticket?branch=` and no `ticketId` (BE contract #3408: today's newest active ticket, or 404
// `noTicket`). The implementer makes these pass and must not edit them.
import { describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError, type Fetch } from '../../../shared/api/client';
import { makeMyQueueTicketData } from '../../../shared/testing/fixtures/my_queue_ticket';
import { readMyQueueTicket } from './queueTicketRead';

const TICKET = makeMyQueueTicketData({
  ticketId: 't-1', branch: 'B01', displayNumber: 'A012', serviceType: 'WALK_IN', status: 'WAITING',
  ticketsAhead: 3, estimateSeconds: 600, estimateLowSeconds: 420, estimateHighSeconds: 780,
  estimateAsOf: '2026-10-19T08:00:00Z',
});

type Reply = { status: number; body: unknown } | 'network';

function server(reply: Reply = { status: 200, body: { version: 1, changed: true, data: TICKET } }) {
  const urls: URL[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string) => {
    urls.push(new URL(input, 'http://localhost'));
    if (reply === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
  });
  return { api: new ApiClient(fetchFn), urls };
}

describe('readMyQueueTicket (C09)', () => {
  it('with a branch and no ticketId, asks my_queue_ticket for that branch with no ticketId and answers the ticket', async () => {
    const s = server();
    const read = await readMyQueueTicket(s.api, 'B01', null);
    expect(read).toEqual({ state: 'ready', data: TICKET });
    expect(s.urls).toHaveLength(1);
    expect(s.urls[0].pathname).toBe('/api/v3/views/my_queue_ticket');
    expect(s.urls[0].searchParams.get('branch')).toBe('B01');
    expect(s.urls[0].searchParams.has('ticketId')).toBe(false);
  });

  it('404 noTicket (no active ticket today) answers empty, not an error', async () => {
    const s = server({ status: 404, body: { error: { code: 'noTicket', message: 'no ticket' } } });
    await expect(readMyQueueTicket(s.api, 'B01', null)).resolves.toEqual({ state: 'empty' });
    expect(s.urls).toHaveLength(1);
  });

  it('a ticketId still reads that ticket, as before', async () => {
    const s = server();
    await expect(readMyQueueTicket(s.api, 'B01', 't-1')).resolves.toEqual({ state: 'ready', data: TICKET });
    expect(s.urls[0].searchParams.get('ticketId')).toBe('t-1');
  });

  it('with no branch, nothing is asked and the answer is empty', async () => {
    const s = server();
    await expect(readMyQueueTicket(s.api, null, null)).resolves.toEqual({ state: 'empty' });
    await expect(readMyQueueTicket(s.api, null, 't-1')).resolves.toEqual({ state: 'empty' });
    expect(s.urls).toHaveLength(0);
  });

  it('401 answers unauthorized', async () => {
    const s = server({ status: 401, body: { error: { code: 'signIn', message: 'sign in' } } });
    await expect(readMyQueueTicket(s.api, 'B01', null)).resolves.toEqual({ state: 'unauthorized' });
  });

  it('a server failure is thrown for the screen to word', async () => {
    const s = server({ status: 500, body: { error: { code: 'internal', message: 'boom' } } });
    await expect(readMyQueueTicket(s.api, 'B01', null)).rejects.toBeInstanceOf(ApiError);
  });
});
