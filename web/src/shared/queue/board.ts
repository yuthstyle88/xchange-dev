// QU-6 (#1354, design `docs/design/blueprint/queue-tickets.md` §4.5; ruling on #1354 issuecomment-6117565689 item 1):
// what the queue card and the admin bar show, derived from `queue_board`. The view answers only WAITING tickets in call
// order, the CALLED/SERVING ones with their teller, and the caller's counter; the count and the "new request" key the
// card's chime and the bar's toast need are read from it here, in one place.

import type { QueueBoardData, QueueBoardWaiting } from '../api/generated/queue_board';

export interface BoardSummary {
  /** How many tickets wait. */
  count: number;
  /** The id of the newest waiting ticket (latest `issuedAt`, then the highest sequence), or `null` when none waits. */
  latest: string | null;
}

function newer(a: QueueBoardWaiting, b: QueueBoardWaiting): boolean {
  const at = Date.parse(a.issuedAt) - Date.parse(b.issuedAt);
  return at !== 0 ? at > 0 : a.sequence > b.sequence;
}

export function summarize(board: QueueBoardData): BoardSummary {
  let newest: QueueBoardWaiting | undefined;
  for (const ticket of board.waiting) {
    if (!newest || newer(ticket, newest)) newest = ticket;
  }
  return { count: board.waiting.length, latest: newest?.id ?? null };
}
