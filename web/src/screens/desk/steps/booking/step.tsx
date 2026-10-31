// Step 2, booking (design xchange-v3-wave3-desk.md §3.2 row "2 Booking", §3.9 case 5, D-10; #543 Part 1).
// `staff` is the signed-in name, read-only. No client-made booking reference: the server numbers the exchange (D-10).
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../../shared/api/client';
import type { QueueServeWalkInInput, QueueTicketDto } from '../../../../shared/api/generated/queue';
import { screenT, t as sharedT } from '../../../../shared/i18n';
import { fieldError } from '../../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../../shared/i18n/worded-error';
import { bangkokToday, type DeskStore } from '../../desk-store';
import { readHeldTicket } from '../../held-ticket';
import type { DeskStep } from '../../steps';
import * as texts from './i18n';

const t = screenT('desk.steps.booking', texts);

const OWN_ERROR_KEYS = {
  outsideServiceHours: 'error.outsideServiceHours',
  queueClosed: 'error.queueClosed',
  noOpenShift: 'error.noOpenShift',
  counterPaused: 'error.counterPaused',
  alreadyServing: 'error.alreadyServing',
} as const satisfies Record<string, keyof typeof texts.th>;

function hasEntryKey(store: DeskStore): boolean {
  const extras = store.entry()?.payloadExtras();
  if (!extras) return false;
  return Boolean(extras.bookingId || extras.onlineBookingId || extras.kioskRequestId);
}

function serveErrorMessage(err: unknown): string {
  if (err instanceof ApiError && !isServerFailure(err)) {
    if (err.code in OWN_ERROR_KEYS) {
      return t(OWN_ERROR_KEYS[err.code as keyof typeof OWN_ERROR_KEYS]);
    }
    if (err.code === 'validationFailed') {
      return fieldError(err, {}).message;
    }
  }
  return wordedError(err);
}

function BookingStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const [error, setError] = createSignal<string | undefined>();
  const [serving, setServing] = createSignal(false);

  async function onSubmit(ev: Event): Promise<void> {
    ev.preventDefault();
    if (serving()) return;
    if (store.fields().name.trim() === '' || store.staff().trim() === '') {
      setError(t('needNameStaff'));
      return;
    }
    if (store.valueDate().trim() === '') {
      setError(sharedT('input.invalid', { field: t('valueDate') }));
      return;
    }
    setError(undefined);
    if (!hasEntryKey(store) && !store.walkInTicket() && !store.heldBooking()) {
      setServing(true);
      try {
        const held = await readHeldTicket(store.api, store.branch(), store.meId(), bangkokToday());
        if (held.kind === 'booking') {
          store.setHeldBooking(held.booking);
          store.go(3);
          return;
        }
        if (held.kind === 'bookingUnmatched') {
          setError(t('error.heldBookingUnmatched', { number: held.displayNumber }));
          return;
        }
        const answer = await store.api.command<QueueServeWalkInInput, QueueTicketDto>(
          'POST',
          '/api/queue/tickets/serve',
          { customerId: store.customerId() || null },
          store.serveKey(),
        );
        store.setWalkInTicket(answer);
      } catch (err) {
        setError(serveErrorMessage(err));
        return;
      } finally {
        setServing(false);
      }
    }
    store.go(3);
  }

  return (
    <form class="desk-form" onSubmit={(ev) => void onSubmit(ev)}>
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
          required
          value={store.valueDate()}
          onInput={(e) => store.setValueDate(e.currentTarget.value)}
        />
      </label>
      <Show when={store.walkInTicket()}>
        {(ticket) => (
          <p class="desk-notice" data-row="walkin-ticket">
            {t('ticket', { number: ticket().displayNumber })}
          </p>
        )}
      </Show>
      <Show when={store.heldBooking()}>
        {(held) => (
          <p class="desk-notice" data-row="held-booking">
            {t('heldBooking', { number: held().displayNumber })}
          </p>
        )}
      </Show>
      <Show when={error()}>
        <p class="desk-error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="desk-actions">
        <button type="button" onClick={() => store.go(1)}>
          {t('back')}
        </button>
        <button type="submit" disabled={serving()}>
          {t('submit')}
        </button>
      </div>
    </form>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 2,
  label: () => t('label'),
  component: (p) => <BookingStep store={p.store} />,
};
