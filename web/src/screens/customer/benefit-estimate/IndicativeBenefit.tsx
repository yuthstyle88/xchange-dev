// XC-BOOKING-ESTIMATE FE-0 (#5395): shared IndicativeBenefit block for the customer's estimate.
import { For, Show, type JSX } from 'solid-js';
import type { PlannerBenefitDto, PlannerBenefitItemDto } from '../../../shared/api/generated/planner_preview';
import { screenT } from '../../../shared/i18n';
import { formatRate } from '../../../shared/ui/rate';
import { en, th, zh } from './i18n';

const t = screenT('customer.benefit-estimate', { th, en, zh });

export interface IndicativeBenefitProps {
  benefit: PlannerBenefitDto | null | undefined;
  slotStartAt?: string | null;
}

export function IndicativeBenefit(props: IndicativeBenefitProps): JSX.Element {
  const estimated = () =>
    props.benefit && props.benefit.state === 'ESTIMATED' ? props.benefit : null;

  const itemValue = (item: PlannerBenefitItemDto): string => {
    if (item.benefitType === 'FEE_WAIVER') {
      return t('feeWaiver');
    }
    if (item.benefitClass === 'REWARD') {
      return `${item.amountThb} THB`;
    }
    return `${formatRate(item.rateBefore, '1', '')} → ${formatRate(item.rateAfter, '1', '')}`;
  };

  const hasTotal = () => {
    const b = estimated();
    return Boolean(b && b.benefitThb !== '0.00');
  };

  const endsBeforeSlot = () => {
    const b = estimated();
    if (!b || !props.slotStartAt) return false;
    const slotTime = Date.parse(props.slotStartAt);
    const validUntilTime = Date.parse(b.validUntil);
    return !Number.isNaN(slotTime) && !Number.isNaN(validUntilTime) && slotTime >= validUntilTime;
  };

  return (
    <Show when={estimated()}>
      {(b) => (
        <section data-row="benefit-estimate">
          <For each={b().items}>
            {(item) => (
              <p data-row="benefit-item">
                {item.label}: {itemValue(item)}
              </p>
            )}
          </For>
          <Show when={hasTotal()}>
            <p data-row="benefit-total">
              {t('total', { amount: b().benefitThb })}
            </p>
          </Show>
          <p data-row="benefit-indicative-note">{t('note')}</p>
          <Show when={endsBeforeSlot()}>
            <p data-row="benefit-ends-before-slot" role="note">
              {t('endsBeforeSlot')}
            </p>
          </Show>
        </section>
      )}
    </Show>
  );
}
