// B06 per-ticket actions (FE-desk skeleton stub, #564): call, recall, skip, start, finish and cancel with a reason, as
// the ticket's status and the caller allow. Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { QueueCommands } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';

/** The board row a set of actions acts on: a WAITING, CALLED or SERVING ticket. */
export interface ActionTicket {
  id: string;
  displayNumber: string;
  /** `WAITING`, `CALLED` or `SERVING`. */
  status: string;
  /** The teller whose counter holds it; `null` while WAITING. */
  tellerId: string | null;
}

export interface TicketActionsProps {
  ticket: ActionTicket;
  me: QueueMe;
  commands: QueueCommands;
  /** Called after any command answered, so the board reads again. */
  onDone(): void;
}

export function TicketActions(_props: TicketActionsProps): JSX.Element {
  return null;
}
