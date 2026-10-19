// B03 benefit lines (FE-desk skeleton stub, #564; design marketing-campaigns.md §10: the quote's `benefits` lists the
// snapshot's `applied`, shape `AppliedBenefit` at api/crates/xc-marketing/src/domain/engine/snapshot.rs:8-25). Rendered
// inside the summary's <dl>. Filled by its own ticket.
import { For, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { th, en } from './i18n';

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

export function BenefitLines(props: { benefits: readonly unknown[] }): JSX.Element {
  const t = screenT('desk.benefits', { th, en });

  const validBenefits = () =>
    (props.benefits ?? []).filter(
      (b): b is AppliedBenefitView => typeof b === 'object' && b !== null,
    );

  return (
    <For each={validBenefits()}>
      {(b) => {
        const label = () => (b.couponId ? `${t('benefit')} (${t('coupon')})` : t('benefit'));
        const value = () => {
          let text: string;
          if (b.benefitClass === 'REWARD') {
            text = `${b.amountThb} THB`;
          } else if (b.feeBefore !== b.feeAfter) {
            text = `${b.feeBefore} → ${b.feeAfter}`;
          } else {
            text = `${b.rateBefore} → ${b.rateAfter}`;
          }
          return b.floorCapped ? `${text} (${t('capped')})` : text;
        };

        return (
          <>
            <dt>{label()}</dt>
            <dd>{value()}</dd>
          </>
        );
      }}
    </For>
  );
}
