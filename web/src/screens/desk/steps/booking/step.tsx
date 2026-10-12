// Step 2, booking (design xchange-v3-wave3-desk.md §3.2 row "2 Booking", §3.9 case 5, D-10; #543 Part 1).
// `staff` is the signed-in name, read-only. No client-made booking reference: the server numbers the exchange (D-10).
import { createSignal, Show, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import type { DeskStep } from '../../steps';
import * as texts from './i18n';

const t = screenT('desk.steps.booking', texts);

function BookingStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const [error, setError] = createSignal<string | undefined>();

  function onSubmit(ev: Event): void {
    ev.preventDefault();
    if (store.fields().name.trim() === '' || store.staff().trim() === '') {
      setError(t('needNameStaff'));
      return;
    }
    setError(undefined);
    store.go(3);
  }

  return (
    <form class="desk-form" onSubmit={onSubmit}>
      <h2>{t('heading')}</h2>
      <label class="desk-field">
        <span>{t('name')}</span>
        <input name="name" readOnly value={store.fields().name} />
      </label>
      <label class="desk-field">
        <span>{t('staff')}</span>
        <input name="staff" readOnly title={t('staffTitle')} value={store.staff()} />
      </label>
      <label class="desk-field">
        <span>{t('valueDate')}</span>
        <input
          name="valueDate"
          type="date"
          value={store.valueDate()}
          onInput={(e) => store.setValueDate(e.currentTarget.value)}
        />
      </label>
      <Show when={error()}>
        <p class="desk-error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="desk-actions">
        <button type="button" onClick={() => store.go(1)}>
          {t('back')}
        </button>
        <button type="submit">{t('submit')}</button>
      </div>
    </form>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 2,
  label: () => t('label'),
  component: (p) => <BookingStep store={p.store} />,
};
