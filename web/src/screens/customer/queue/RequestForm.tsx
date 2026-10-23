// QW-2c (kiosk-shared-phone §5r F7; row U2's form; §5r.4a): the request form.
// Renders select[name="type"] (BUY | SELL), select[name="currency"] (one option per `rates` code),
// input[name="amount"] and button[data-action="submit-request"]. The button hands `{type, currency, amount}` to
// `onSubmit`; it is disabled while `busy`.
// Classes: xc-queue-form, xc-queue-field, xc-queue-label, xc-queue-select, xc-queue-input, xc-queue-submit.
// Texts (i18n.ts): type, buy, sell, currency, amount, submit, submitting.
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { atMost, checkDecimal, MONEY } from '../../../shared/ui/bounded';
import * as table from './i18n';
import type { QueueRate, RequestFormValue, TradeType } from './queue-api';

const t = screenT('customer.queue', table);
const QUEUE_AMOUNT_BOUND = atMost(MONEY, '10000000');

export interface RequestFormProps {
  rates: QueueRate[];
  busy: boolean;
  onSubmit: (value: RequestFormValue) => void;
}

export function RequestForm(props: RequestFormProps): JSX.Element {
  const [type, setType] = createSignal<TradeType>('BUY');
  const [selectedCurrency, setSelectedCurrency] = createSignal<string>('');
  const [amount, setAmount] = createSignal<string>('');
  const [amountInvalid, setAmountInvalid] = createSignal(false);

  const amountError = () =>
    amountInvalid()
      ? fieldError(
          new ApiError({ status: 400, code: 'validationFailed', field: 'amount', message: '' }),
          { amount: t('amount') },
        ).message
      : null;

  const currentCurrency = () => {
    const cur = selectedCurrency();
    if (cur && props.rates.some((r) => r.code === cur)) {
      return cur;
    }
    return props.rates[0]?.code ?? '';
  };

  let inTick = false;
  const handleSubmit = (e?: Event) => {
    if (e) {
      e.preventDefault();
    }
    if (props.busy || inTick) return;
    inTick = true;
    queueMicrotask(() => {
      inTick = false;
    });
    const rawAmount = amount();
    const checked = checkDecimal(rawAmount, QUEUE_AMOUNT_BOUND);
    if (!checked.ok || /^0(?:\.0*)?$/.test(rawAmount)) {
      setAmountInvalid(true);
      return;
    }
    setAmountInvalid(false);
    props.onSubmit({
      type: type(),
      currency: currentCurrency(),
      amount: rawAmount,
    });
  };

  return (
    <form class="xc-queue-form" onSubmit={handleSubmit}>
      <div class="xc-queue-field">
        <label class="xc-queue-label" for="request-type">
          {t('type')}
        </label>
        <select
          id="request-type"
          name="type"
          class="xc-queue-select"
          value={type()}
          onChange={(e) => setType(e.currentTarget.value as TradeType)}
          onInput={(e) => setType(e.currentTarget.value as TradeType)}
        >
          <option value="BUY" label={t('buy')}>
            {t('buy')}
          </option>
          <option value="SELL" label={t('sell')}>
            {t('sell')}
          </option>
        </select>
      </div>

      <div class="xc-queue-field">
        <label class="xc-queue-label" for="request-currency">
          {t('currency')}
        </label>
        <select
          id="request-currency"
          name="currency"
          class="xc-queue-select"
          value={currentCurrency()}
          onChange={(e) => setSelectedCurrency(e.currentTarget.value)}
          onInput={(e) => setSelectedCurrency(e.currentTarget.value)}
        >
          <For each={props.rates}>
            {(rate) => {
              const name = rate.name || rate.nameEn || '';
              const label = name ? `${rate.code} - ${name}` : rate.code;
              return (
                <option value={rate.code} label={label}>
                  {label}
                </option>
              );
            }}
          </For>
        </select>
      </div>

      <div class="xc-queue-field">
        <label class="xc-queue-label" for="request-amount">
          {t('amount')}
        </label>
        <input
          id="request-amount"
          name="amount"
          type="text"
          inputmode="decimal"
          class="xc-queue-input"
          value={amount()}
          onInput={(e) => setAmount(e.currentTarget.value)}
          onChange={(e) => setAmount(e.currentTarget.value)}
        />
        <Show when={amountError()}>
          {(msg) => <span data-field-error="amount">{msg()}</span>}
        </Show>
      </div>

      <button
        type="submit"
        data-action="submit-request"
        class="xc-queue-submit"
        disabled={props.busy}
        onClick={handleSubmit}
      >
        {props.busy ? t('submitting') : t('submit')}
      </button>
    </form>
  );
}
