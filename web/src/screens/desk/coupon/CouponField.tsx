// B03 the desk's coupon input (FE-desk skeleton stub, #564; design xchange-v3-wave3-desk.md D-5, marketing-campaigns.md
// D-9, §10). Placed in the summary aside, not in step 0: the rate step's provided tests build a stub store without the
// coupon members (steps/rate/step.test.tsx:57-89). Filled by its own ticket.
import { Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import type { DeskStore } from '../desk-store';
import { en, th } from './i18n';

const t = screenT('desk.coupon', { th, en });

export function CouponField(props: { store: DeskStore }): JSX.Element {
  const locked = () => props.store.entry()?.locked === true;
  const couponError = () => {
    const e = props.store.quoteError();
    return e?.field === 'couponCode' ? e.message : undefined;
  };

  return (
    <label class="desk-field">
      <span>{t('couponCode')}</span>
      <input
        name="couponCode"
        autocomplete="off"
        value={props.store.couponCode()}
        readOnly={locked()}
        onInput={(e) => props.store.setCouponCode(e.currentTarget.value)}
      />
      <Show when={couponError()}>
        {(msg) => (
          <p class="desk-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
    </label>
  );
}
