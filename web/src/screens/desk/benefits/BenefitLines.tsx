// B03 benefit lines (FE-desk skeleton stub, #564; design marketing-campaigns.md §10: the quote's `benefits` lists the
// snapshot's `applied`, shape `AppliedBenefit` at api/crates/xc-marketing/src/domain/engine/snapshot.rs:8-25). Rendered
// inside the summary's <dl>. Filled by its own ticket.
import type { JSX } from 'solid-js';

/** One applied benefit, as the server serialises `AppliedBenefit` (camelCase; money and rates as strings). */
export interface AppliedBenefitView {
  ruleId: string;
  versionId: string;
  benefitType: string;
  /** `PRICE` or `REWARD`. */
  benefitClass: string;
  couponId?: string;
  reservationId?: string;
  amountThb: string;
  rateBefore: string;
  rateAfter: string;
  feeBefore: string;
  feeAfter: string;
  floorCapped: boolean;
}

export function BenefitLines(_props: { benefits: readonly unknown[] }): JSX.Element {
  return null;
}
