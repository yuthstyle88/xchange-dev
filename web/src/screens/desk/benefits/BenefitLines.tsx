// B03 benefit lines (FE-desk skeleton stub, #564; design marketing-campaigns.md §10: the quote's `benefits` lists the
// snapshot's `applied`, shape `AppliedBenefit` at api/crates/xc-marketing/src/domain/engine/snapshot.rs:8-25). Rendered
// inside the summary's <dl>. Filled by its own ticket.
import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import type { QuoteBenefitBasis, QuoteBenefitDto } from '../../../shared/api/generated/quote';
import { th, en } from './i18n';

export function BenefitLines(props: {
  benefits: readonly QuoteBenefitDto[];
  basis?: QuoteBenefitBasis;
}): JSX.Element {
  const t = screenT('desk.benefits', { th, en });

  return (
    <For each={props.benefits}>
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
            <dd>
              {value()}
              <Show when={props.basis === 'INDICATIVE'}>
                {' '}
                <span class="desk-badge" data-row="benefit-indicative">
                  {t('indicative')}
                </span>
              </Show>
            </dd>
          </>
        );
      }}
    </For>
  );
}
