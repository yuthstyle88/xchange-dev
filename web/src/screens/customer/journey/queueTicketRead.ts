// C09: read the customer's queue ticket for the journey screen (design queue-tickets.md §4.4 `:164`;
// section-b-gaps.md S5; BE contract #3408: `my_queue_ticket?branch=` without `ticketId` answers today's newest
// active ticket, or 404 `noTicket`).
// SKELETON (feat/fe-customer-2): this is today's read, moved out of JourneyScreen.tsx unchanged; the function
// ticket makes the no-`ticketId` read. The signature stays.
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';

export type QueueTicketRead =
  | { state: 'ready'; data: MyQueueTicketData }
  | { state: 'empty' }
  | { state: 'unauthorized' };

/** Any other failure is thrown for the screen to word. */
export async function readMyQueueTicket(
  api: ApiClient,
  branch: string | null,
  ticketId: string | null,
): Promise<QueueTicketRead> {
  if (!branch) return { state: 'empty' };
  try {
    const answer = await api.view<MyQueueTicketData, { ticketId: string }>('my_queue_ticket', {
      branch,
      filter: ticketId ? { ticketId } : undefined,
    });
    const data = answer.data;
    return data && data.ticketId ? { state: 'ready', data } : { state: 'empty' };
  } catch (err: unknown) {
    if ((err as { status?: number })?.status === 401) return { state: 'unauthorized' };
    if (
      err instanceof ApiError &&
      ((err.status === 404 && (err.code === 'noTicket' || err.code === 'httpError')) ||
        (err.status === 403 && err.code === 'httpError'))
    ) {
      return { state: 'empty' };
    }
    throw err;
  }
}
