// Step 0, rate and amount (design xchange-v3-wave3-desk.md §3.2 row 0, D-4, D-12; #539 Part 2 §7–§9).
// The rate is the server's quote, shown as answered beside its unit size; it is never typed (D-12). No arithmetic (W7).
import { createSignal, For, Show, type JSX } from 'solid-js';
import { screenT, t as sharedT } from '../../../../shared/i18n';
import { checkDecimal, foreignAmount } from '../../../../shared/ui/bounded';
import type { DeskStore, DeskType } from '../../desk-store';
import type { DeskStep } from '../../steps';
import * as texts from './i18n';

const t = screenT('desk.steps.rate', texts);
// shortcut: DeskRate has no minorUnit so client checks max scale 3, upgrade when desk_config exposes minorUnit
const AMOUNT_BOUND = foreignAmount(3);

function RateStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const [refusal, setRefusal] = createSignal<string | undefined>();
  const [badAmount, setBadAmount] = createSignal(false);
  const locked = () => store.entry()?.locked === true;
  const shiftOpen = () => store.config()?.shiftOpen === true;
  const line = () => store.quote()?.lines[0];
  const amountError = () => {
    if (badAmount() && store.amount() === '') {
      return sharedT('input.invalid', { field: t('amount') });
    }
    const e = store.quoteError();
    return e?.field === 'quantity' ? e.message : undefined;
  };
  const otherError = () => {
    const e = store.quoteError();
    return e && e.field !== 'quantity' && e.field !== 'currency' && e.field !== 'couponCode' && e.field !== 'feeThb'
      ? e.message
      : undefined;
  };
  const currencies = () => {
    const codes = (store.config()?.rates ?? []).map((r) => r.code);
    const current = store.currency();
    return current !== '' && !codes.includes(current) ? [current, ...codes] : codes;
  };

  function onSubmit(ev: Event): void {
    ev.preventDefault();
    setRefusal(undefined);
    const cfgErr = store.configError();
    if (cfgErr) {
      setRefusal(cfgErr);
      return;
    }
    const err = store.quoteError();
    if (!shiftOpen() || err?.field === 'currency') {
      setRefusal(t('needShiftAndRate'));
      return;
    }
    if (!store.quote()) {
      if (!err && !badAmount()) setRefusal(t('needShiftAndRate'));
      return;
    }
    store.go(1);
  }

  return (
    <form class="desk-form" onSubmit={onSubmit}>
      <h2>{t('heading')}</h2>
      <Show when={store.configError() || !shiftOpen()}>
        <p class="desk-notice">{store.configError() ?? t('noShift')}</p>
      </Show>

      <label class="desk-field">
        <span>{t('type')}</span>
        <select
          name="type"
          value={store.type()}
          disabled={locked()}
          onChange={(e) => store.setType(e.currentTarget.value as DeskType)}
        >
          <option value="BUY">{t('typeBuy')}</option>
          <option value="SELL">{t('typeSell')}</option>
        </select>
      </label>

      <label class="desk-field">
        <span>{t('currency')}</span>
        <select
          name="currency"
          value={store.currency()}
          disabled={locked()}
          onChange={(e) => store.setCurrency(e.currentTarget.value)}
        >
          <For each={currencies()}>{(code) => <option value={code}>{code}</option>}</For>
        </select>
      </label>

      <label class="desk-field">
        <span>{t('amount')}</span>
        <input
          name="amount"
          inputMode="decimal"
          autocomplete="off"
          value={store.amount()}
          readOnly={locked()}
          onInput={(e) => {
            const v = e.currentTarget.value;
            if (v !== '' && !checkDecimal(v, AMOUNT_BOUND).ok) {
              setBadAmount(true);
              store.setAmount('');
              e.currentTarget.value = v;
              return;
            }
            setBadAmount(false);
            store.setAmount(v);
          }}
        />
        <Show when={amountError()}>
          <span class="desk-error">{amountError()}</span>
        </Show>
      </label>

      <div class="desk-rate-line">
        <span>{t('rate')}</span>
        <Show when={line()}>
          {(l) => (
            <strong>
              {l().finalRate} THB {t('perUnit', { unit: l().unitSize, code: l().currency })}
            </strong>
          )}
        </Show>
        <Show when={store.quoting()}>
          <span class="desk-notice">{t('quoting')}</span>
        </Show>
      </div>

      <label class="desk-field">
        <span>{t('referenceRate')}</span>
        <input
          name="referenceRate"
          inputMode="decimal"
          autocomplete="off"
          value={store.referenceRate()}
          readOnly={locked()}
          onInput={(e) => store.setReferenceRate(e.currentTarget.value)}
        />
      </label>

      <Show when={otherError()}>
        <p class="desk-error">{otherError()}</p>
      </Show>
      <Show when={refusal()}>
        <p class="desk-error" role="alert">
          {refusal()}
        </p>
      </Show>

      <div class="desk-actions">
        <button type="submit">{t('next')}</button>
      </div>
    </form>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 0,
  label: () => t('label'),
  component: (p) => <RateStep store={p.store} />,
};
