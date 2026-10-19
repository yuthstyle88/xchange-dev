// B03 the desk's coupon input (FE-desk skeleton stub, #564; design xchange-v3-wave3-desk.md D-5, marketing-campaigns.md
// D-9, §10). Placed in the summary aside, not in step 0: the rate step's provided tests build a stub store without the
// coupon members (steps/rate/step.test.tsx:57-89). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { DeskStore } from '../desk-store';

export function CouponField(_props: { store: DeskStore }): JSX.Element {
  return null;
}
