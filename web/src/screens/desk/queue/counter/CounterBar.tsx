// B06 counter bar (FE-desk skeleton stub, #564): issue a walk-in ticket, call next, pause or resume my counter, and open
// or close the branch queue (BranchDayOperate only). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { QueueStore } from '../../../../shared/queue/queue-store';
import type { QueueCommands } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';

export interface CounterBarProps {
  store: QueueStore;
  me: QueueMe;
  commands: QueueCommands;
}

export function CounterBar(_props: CounterBarProps): JSX.Element {
  return null;
}
