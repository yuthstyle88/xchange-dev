// B06 board (FE-desk skeleton stub, #564): the WAITING tickets in call order and the CALLED/SERVING ones with their
// teller, from `queue_board`. Each row mounts `TicketActions`. Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { QueueStore } from '../../../../shared/queue/queue-store';
import type { QueueCommands } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';

export interface BoardProps {
  store: QueueStore;
  me: QueueMe;
  commands: QueueCommands;
}

export function Board(_props: BoardProps): JSX.Element {
  return null;
}
