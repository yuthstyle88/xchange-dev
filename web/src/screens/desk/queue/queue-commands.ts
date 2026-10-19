// B06 queue commands (design docs/design/blueprint/queue-tickets.md §5; routes at
// api/src/presentation/http/v3/commands/queue.rs:236-492). One keyed write per route, so a retry of the same body reuses
// its Idempotency-Key. The parts call these; none writes a route of its own.
import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  QueueCancelInput,
  QueueCounterDto,
  QueueNoInput,
  QueueStateDto,
  QueueTicketDto,
  QueueTicketInput,
} from '../../../shared/api/generated/queue';

/** The per-ticket verbs of `POST /api/queue/tickets/{id}/<verb>` (`queue.rs:335-351`). */
export type TicketVerb = 'call' | 'recall' | 'skip' | 'start' | 'finish';

export interface QueueCommands {
  /** `POST /api/queue/tickets`: a walk-in ticket issued at the desk (`queue.rs:236`). */
  issueWalkIn(): Promise<QueueTicketDto>;
  /** `POST /api/queue/call-next` (`queue.rs:264`). */
  callNext(): Promise<QueueTicketDto>;
  /** `POST /api/queue/tickets/{id}/<verb>`. */
  ticket(verb: TicketVerb, id: string): Promise<QueueTicketDto>;
  /** `POST /api/queue/tickets/{id}/cancel` with a reason (`queue.rs:362`). */
  cancel(id: string, reason: string): Promise<QueueTicketDto>;
  /** `POST /api/queue/counter/pause` / `resume` (`queue.rs:403`, `:432`). */
  counter(verb: 'pause' | 'resume'): Promise<QueueCounterDto>;
  /** `POST /api/queue/open` / `close` (`queue.rs:463`, `:492`); `BranchDayOperate` only. */
  queue(verb: 'open' | 'close'): Promise<QueueStateDto>;
}

const enc = encodeURIComponent;

export function queueCommands(api: ApiClient): QueueCommands {
  const walkIn = keyedWrite<QueueNoInput, QueueTicketDto>(api, 'POST', '/api/queue/tickets');
  const next = keyedWrite<QueueNoInput, QueueTicketDto>(api, 'POST', '/api/queue/call-next');
  const verbs = new Map<TicketVerb, ReturnType<typeof keyedWrite<QueueTicketInput, QueueTicketDto>>>();
  const cancelWrite = keyedWrite<QueueCancelInput, QueueTicketDto>(
    api,
    'POST',
    (b) => `/api/queue/tickets/${enc(b.id)}/cancel`,
  );
  const counterWrites = {
    pause: keyedWrite<QueueNoInput, QueueCounterDto>(api, 'POST', '/api/queue/counter/pause'),
    resume: keyedWrite<QueueNoInput, QueueCounterDto>(api, 'POST', '/api/queue/counter/resume'),
  };
  const queueWrites = {
    open: keyedWrite<QueueNoInput, QueueStateDto>(api, 'POST', '/api/queue/open'),
    close: keyedWrite<QueueNoInput, QueueStateDto>(api, 'POST', '/api/queue/close'),
  };

  return {
    issueWalkIn: () => walkIn.send({}),
    callNext: () => next.send({}),
    ticket(verb, id) {
      let w = verbs.get(verb);
      if (!w) {
        w = keyedWrite<QueueTicketInput, QueueTicketDto>(api, 'POST', (b) => `/api/queue/tickets/${enc(b.id)}/${verb}`);
        verbs.set(verb, w);
      }
      return w.send({ id });
    },
    cancel: (id, reason) => cancelWrite.send({ id, reason }),
    counter: (verb) => counterWrites[verb].send({}),
    queue: (verb) => queueWrites[verb].send({}),
  };
}
