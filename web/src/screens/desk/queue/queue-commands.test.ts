// W3 (#4039): pin queueCommands routes and idempotency keys.
import { describe, expect, it, vi } from 'vitest';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { queueCommands, type TicketVerb } from './queue-commands';

function fakeApi() {
  const command = vi.fn<
    (method: string, path: string, body: unknown, key: string) => Promise<any>
  >(async () => ({}));
  return { api: { command } as unknown as ApiClient, command };
}

describe('queueCommands (#4039)', () => {
  it('1. ticket("call", "a/b") encodes the ticket id in the path', async () => {
    const { api, command } = fakeApi();
    await queueCommands(api).ticket('call', 'a/b');
    expect(command).toHaveBeenCalledTimes(1);
    const [method, path, body, key] = command.mock.calls[0]!;
    expect(method).toBe('POST');
    expect(path).toBe('/api/queue/tickets/a%2Fb/call');
    expect(body).toEqual({ id: 'a/b' });
    expect(typeof key).toBe('string');
    expect(key.length).toBeGreaterThan(0);
  });

  it('2. issueWalkIn, callNext, cancel, counter pause/resume, queue open/close, and ticket verbs each send their own path and body', async () => {
    const { api, command } = fakeApi();
    const cmds = queueCommands(api);

    await cmds.issueWalkIn();
    await cmds.callNext();
    await cmds.cancel('t/1', 'no-show');
    await cmds.counter('pause');
    await cmds.counter('resume');
    await cmds.queue('open');
    await cmds.queue('close');
    for (const verb of ['call', 'recall', 'skip', 'start', 'finish'] as TicketVerb[]) {
      await cmds.ticket(verb, 't-9');
    }

    const calls = command.mock.calls.map(([method, path, body]) => [method, path, body]);
    expect(calls).toEqual([
      ['POST', '/api/queue/tickets', {}],
      ['POST', '/api/queue/call-next', {}],
      ['POST', '/api/queue/tickets/t%2F1/cancel', { id: 't/1', reason: 'no-show' }],
      ['POST', '/api/queue/counter/pause', {}],
      ['POST', '/api/queue/counter/resume', {}],
      ['POST', '/api/queue/open', {}],
      ['POST', '/api/queue/close', {}],
      ['POST', '/api/queue/tickets/t-9/call', { id: 't-9' }],
      ['POST', '/api/queue/tickets/t-9/recall', { id: 't-9' }],
      ['POST', '/api/queue/tickets/t-9/skip', { id: 't-9' }],
      ['POST', '/api/queue/tickets/t-9/start', { id: 't-9' }],
      ['POST', '/api/queue/tickets/t-9/finish', { id: 't-9' }],
    ]);
  });

  it('3. callNext rejects 5xx, then retried: reuses the same key', async () => {
    const { api, command } = fakeApi();
    const cmds = queueCommands(api);
    command.mockRejectedValueOnce(
      new ApiError({ status: 503, code: 'serverError', message: 'unavailable' }),
    );

    await expect(cmds.callNext()).rejects.toMatchObject({ status: 503 });
    await cmds.callNext();

    expect(command).toHaveBeenCalledTimes(2);
    const key1 = command.mock.calls[0]![3];
    const key2 = command.mock.calls[1]![3];
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
  });

  it('4. a 4xx reject, then retried: generates a new key', async () => {
    const { api, command } = fakeApi();
    const cmds = queueCommands(api);
    command.mockRejectedValueOnce(
      new ApiError({ status: 409, code: 'nothingWaiting', message: 'empty' }),
    );

    await expect(cmds.callNext()).rejects.toMatchObject({ status: 409 });
    await cmds.callNext();

    expect(command).toHaveBeenCalledTimes(2);
    const key1 = command.mock.calls[0]![3];
    const key2 = command.mock.calls[1]![3];
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
  });

  it('5. two different bodies: use different keys', async () => {
    const { api, command } = fakeApi();
    const cmds = queueCommands(api);
    command.mockRejectedValue(
      new ApiError({ status: 500, code: 'serverError', message: 'boom' }),
    );

    await expect(cmds.cancel('t-1', 'left')).rejects.toMatchObject({ status: 500 });
    await expect(cmds.cancel('t-2', 'left')).rejects.toMatchObject({ status: 500 });

    const key1 = command.mock.calls[0]![3];
    const key2 = command.mock.calls[1]![3];
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
  });
});
