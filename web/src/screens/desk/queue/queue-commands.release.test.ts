// #5569 (DESK-R4 T4): `release` is a ticket verb, posted as POST /api/queue/tickets/{id}/release with body {id}
// (queue-tickets.md §4.6). The typed line below also fails `tsc` until `TicketVerb` has it.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken.
import { describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../../shared/api/client';
import { queueCommands, type TicketVerb } from './queue-commands';

describe('queueCommands release (#5569)', () => {
  it('ticket("release", id) posts the release route with the id', async () => {
    const command = vi.fn<(method: string, path: string, body: unknown, key: string) => Promise<unknown>>(async () => ({}));
    const verb: TicketVerb = 'release';
    await queueCommands({ command } as unknown as ApiClient).ticket(verb, 't/9');
    expect(command).toHaveBeenCalledTimes(1);
    const [method, path, body] = command.mock.calls[0]!;
    expect(method).toBe('POST');
    expect(path).toBe('/api/queue/tickets/t%2F9/release');
    expect(body).toEqual({ id: 't/9' });
  });
});
