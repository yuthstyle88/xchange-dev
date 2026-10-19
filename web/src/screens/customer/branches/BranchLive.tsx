// C08: one branch card's live status — waiting, counters open, wait estimate (branch_service_status, public)
// and today's free booking slots (booking_slots, public) (design customer-app.md §4.5).
// SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { Fetch } from '../../../shared/api/client';

export interface BranchLiveProps {
  /** The branch's public code, which is its id. */
  branch: string;
  /** `PublicBranchDto.bookingEnabled`: no slots are read for a branch that takes no bookings. */
  bookingEnabled: boolean;
  /** The screen's fetch, so tests mock at the fetch level. */
  fetch: Fetch;
}

export function BranchLive(_props: BranchLiveProps): JSX.Element {
  return null;
}
