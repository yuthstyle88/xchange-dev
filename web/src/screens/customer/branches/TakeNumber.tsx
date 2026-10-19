// C08: take a queue number from the app, `queueTicketTake` `POST /api/app/queue/tickets` `{branch}`
// (Blueprint C08 "จองหรือรับคิว"; section-b-gaps.md S7; BE contract #3410).
// SKELETON STUB (feat/fe-customer-2): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { Fetch } from '../../../shared/api/client';

export interface TakeNumberProps {
  /** The branch's public code, which is its id. */
  branch: string;
  /** `PublicBranchDto.queueOpen`: no button when the queue is closed. */
  queueOpen: boolean;
  /** The screen's fetch, so tests mock at the fetch level. */
  fetch: Fetch;
}

export function TakeNumber(_props: TakeNumberProps): JSX.Element {
  return null;
}
