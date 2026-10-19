// The summary aside (design xchange-v3-wave3-desk.md §3.2 "Summary aside", D-2, D-4): the server's quote, shown
// unchanged. No receipt before the save (D-2) and no figure the browser computed (W7).
import { Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import type { DeskStore, TransactionState } from './desk-store';
import * as screenI18n from './i18n';
import { BenefitLines } from './benefits/BenefitLines';
import { CustomerLoyalty } from './loyalty/CustomerLoyalty';

const STATE_KEYS: Record<TransactionState, 'stateDraft' | 'statePending' | 'stateConfirmed' | 'stateCompleted' | 'stateCancelled'> = {
  DRAFT: 'stateDraft',
  PENDING_APPROVAL: 'statePending',
  CONFIRMED: 'stateConfirmed',
  COMPLETED: 'stateCompleted',
  CANCELLED: 'stateCancelled',
};

export function Summary(props: { store: DeskStore }): JSX.Element {
  const t = screenT('desk', screenI18n);
  const line = () => props.store.quote()?.lines[0];
  const direction = () => (props.store.type() === 'SELL' ? t('shopReceives') : t('shopPays'));

  return (
    <div class="desk-summary">
      <h2>{t('summary')}</h2>
      <dl>
        <dt>{direction()}</dt>
        <dd>
          {props.store.amount()} {props.store.currency()}
        </dd>
        <Show when={line()}>
          {(l) => (
            <>
              <dt>{t('bookedRate')}</dt>
              <dd>
                {l().finalRate} {t('perUnit', { unit: l().unitSize, code: l().currency })}
              </dd>
              <dt>{t('beforeFee')}</dt>
              <dd>{l().thbAmount} THB</dd>
              <BenefitLines benefits={props.store.quote()?.benefits ?? []} />
              <Show when={props.store.step() >= 3}>
                <dt>{t('fee')}</dt>
                <dd>{l().feeThb}</dd>
                <dt>{t('net', { direction: direction() })}</dt>
                <dd>{l().settleThb} THB</dd>
              </Show>
            </>
          )}
        </Show>
        <Show when={props.store.customerId()}>
          <dt>{t('customer')}</dt>
          <dd>{props.store.customerId()}</dd>
        </Show>
      </dl>
      <CustomerLoyalty store={props.store} />
      <Show when={props.store.quoting()}>
        <p class="desk-notice">{t('recalculating')}</p>
      </Show>
      <Show when={props.store.transactionState()}>
        {(s) => <p class="desk-notice">{t(STATE_KEYS[s()])}</p>}
      </Show>
    </div>
  );
}
