// Step 3, payment (design xchange-v3-wave3-desk.md §3.2 rows "3 Payment", "3 payments", "3 cash count", §3.6 B2–B3,
// D-4, D-7, §5.4; #543 Part 1). The net is the quote line's `settleThb` as answered (D-4); the save is `store.submit()`.
import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import type { DeskStep } from '../../steps';
import { isOpen, METHODS, ruleFor } from './channels';
import * as texts from './i18n';

const t = screenT('desk.steps.payment', texts);

/** 15 MB, v2's limit (dh:126). */
const MAX_FILE_BYTES = 15_728_640;
const SLIP_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'application/pdf'];
const SLIP_ACCEPT = '.jpg,.jpeg,.png,.webp,.pdf,image/jpeg,image/png,image/webp,application/pdf';
/** v2 sends Cash's reference as an em dash (dh:117). */
const CASH_REF = '—';

function slipRefused(file: File): boolean {
  const known = SLIP_TYPES.includes(file.type) || /\.(jpe?g|png|webp|pdf)$/i.test(file.name);
  const heic = file.type === 'image/heic' || file.type === 'image/heif' || /\.(heic|heif)$/i.test(file.name);
  return heic || !known || file.size === 0 || file.size > MAX_FILE_BYTES;
}

function PaymentStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  let slipEl: HTMLInputElement | undefined;
  const [error, setError] = createSignal<string | undefined>();
  const [slipError, setSlipError] = createSignal<string | undefined>();

  const rules = () => store.config()?.paymentMethods;
  const noRules = () => !rules() || rules()!.length === 0;
  const open = (m: string) => isOpen(rules(), m, store.type());
  const isCash = () => store.method() === 'Cash';
  const channelOff = () => !noRules() && store.method() !== '' && !open(store.method());
  const net = () => store.quote()?.lines[0]?.settleThb ?? '';
  const instruction = () => ruleFor(rules(), store.method())?.instruction ?? '';
  const entry = () => store.entry();

  function choose(method: string): void {
    store.setMethod(method);
    if (method !== 'Cash' && store.ref() === CASH_REF) {
      store.setRef('');
    }
    const cur = store.account().trim();
    const autoFilled = cur === '' || (rules() ?? []).some((r) => r.account.trim() !== '' && r.account.trim() === cur);
    if (autoFilled) {
      const next = ruleFor(rules(), method)?.account ?? '';
      if (next !== '' || store.account() !== '') {
        store.setAccount(next);
      }
    }
  }

  onMount(() => {
    if (store.method() === '') {
      const first = METHODS.find((m) => open(m));
      if (first) choose(first);
    }
  });

  function onSlip(e: Event & { currentTarget: HTMLInputElement }): void {
    const file = e.currentTarget.files?.[0];
    setSlipError(undefined);
    if (!file) return;
    if (slipRefused(file)) {
      setSlipError(t('slipBad'));
      e.currentTarget.value = '';
      return;
    }
    store.setSlip(file);
  }

  function onCancel(): void {
    if (window.confirm(t('cancelConfirm'))) {
      store.go(0);
    }
  }

  function onSubmit(ev: Event): void {
    ev.preventDefault();
    setError(undefined);
    if (noRules()) {
      setError(t('noRules'));
      return;
    }
    if (channelOff()) {
      setError(t('channelOff'));
      return;
    }
    if (!store.paymentChecked()) return;
    const refusal = entry()?.refusal();
    if (refusal) {
      setError(refusal);
      return;
    }
    const needsRef = !isCash();
    if (
      store.method() === '' ||
      store.account().trim() === '' ||
      !store.passport() ||
      net() === '' ||
      (needsRef && (store.ref().trim() === '' || !store.slip()))
    ) {
      setError(t('incomplete'));
      return;
    }
    if (isCash()) {
      // desk-devices.js:4: the count is confirmed before the cash is handed over
      if (!window.confirm(t('countConfirm'))) {
        setError(t('countNotConfirmed'));
        return;
      }
      store.setRef(CASH_REF);
      store.setCountConfirmed(true);
    } else {
      store.setCountConfirmed(false);
    }
    store.go(4);
    void store.submit();
  }

  return (
    <form class="desk-form" onSubmit={onSubmit}>
      <h2>{t('heading')}</h2>
      <p>
        <strong>{t('net', { net: net() })}</strong>
      </p>
      <Show when={noRules()}>
        <p class="desk-notice">{t('noRules')}</p>
      </Show>

      <label class="desk-field">
        <span>{t('method')}</span>
        <select name="method" value={store.method()} onChange={(e) => choose(e.currentTarget.value)}>
          <For each={METHODS}>
            {(m) => (
              <option
                value={m}
                disabled={!noRules() && !open(m)}
                hidden={!noRules() && !open(m) && store.method() !== m}
              >
                {m}
              </option>
            )}
          </For>
        </select>
        <Show when={instruction()}>
          <span class="desk-notice">{instruction()}</span>
        </Show>
        <Show when={channelOff()}>
          <span class="desk-error">{t('channelOff')}</span>
        </Show>
      </label>

      <label class="desk-field">
        <span>{t('ref')}</span>
        <input
          name="ref"
          autocomplete="off"
          value={store.ref()}
          disabled={isCash()}
          onInput={(e) => store.setRef(e.currentTarget.value)}
        />
      </label>
      <label class="desk-field">
        <span>{t('account')}</span>
        <input name="account" autocomplete="off" value={store.account()} onInput={(e) => store.setAccount(e.currentTarget.value)} />
      </label>
      <label class="desk-field">
        <span>{t('fee')}</span>
        <input name="fee" inputMode="decimal" autocomplete="off" value={store.fee()} onInput={(e) => store.setFee(e.currentTarget.value)} />
      </label>
      <label class="desk-field">
        <span>{t('bankFee')}</span>
        <input
          name="bankFee"
          inputMode="decimal"
          autocomplete="off"
          value={store.bankFee()}
          onInput={(e) => store.setBankFee(e.currentTarget.value)}
        />
      </label>
      <label class="desk-field">
        <span>{t('notes')}</span>
        <textarea name="notes" value={store.notes()} onInput={(e) => store.setNotes(e.currentTarget.value)} />
      </label>
      <label class="desk-field">
        <span>{t('slip')}</span>
        <input ref={slipEl} type="file" name="slip" accept={SLIP_ACCEPT} onChange={onSlip} />
        <Show when={slipError()}>
          <span class="desk-error">{slipError()}</span>
        </Show>
        <Show when={store.slip()}>
          {(file) => (
            <span class="desk-notice">
              {file().name}{' '}
              <button
                type="button"
                onClick={() => {
                  store.setSlip(undefined);
                  if (slipEl) slipEl.value = '';
                }}
              >
                {t('removeSlip')}
              </button>
            </span>
          )}
        </Show>
      </label>

      <Show when={entry()?.identityLabel}>
        {(label) => (
          <label class="desk-check">
            <input
              type="checkbox"
              name="identityConfirmed"
              checked={entry()?.identityConfirmed() ?? false}
              onChange={(e) => entry()?.setIdentityConfirmed(e.currentTarget.checked)}
            />
            <span>{label()}</span>
          </label>
        )}
      </Show>

      <label class="desk-check">
        <input
          type="checkbox"
          name="paymentChecked"
          checked={store.paymentChecked()}
          onChange={(e) => store.setPaymentChecked(e.currentTarget.checked)}
        />
        <span>{t('checked', { net: net() })}</span>
      </label>

      <Show when={error()}>
        <p class="desk-error" role="alert">
          {error()}
        </p>
      </Show>

      <div class="desk-actions">
        <button type="button" onClick={onCancel}>
          {t('cancel')}
        </button>
        <button type="submit" disabled={!store.paymentChecked() || channelOff() || noRules()}>
          {t('submit')}
        </button>
      </div>
    </form>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 3,
  label: () => t('label'),
  component: (p) => <PaymentStep store={p.store} />,
};
