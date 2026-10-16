// APP-17: C03 Customer My Bookings screen (design customer-app.md §4.5, #987).

import {
  createSignal,
  onMount,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  BookingCancelInput,
  BookingCancelOutput,
  BookingRescheduleInput,
  BookingRescheduleOutput,
} from '../../../shared/api/generated/bookings';
import type {
  BookingSlotItemDto,
  BookingSlotsData,
} from '../../../shared/api/generated/booking_slots';
import type {
  MyBookingItemDto,
  MyBookingsData,
} from '../../../shared/api/generated/my_bookings';
import { locale, screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { QrCode } from '../../../shared/ui/QrCode';
import * as table from './i18n';
import './bookings.css';

const t = screenT('customer.bookings', table);

function getStatusLabel(status: string): string {
  const key = `status_${status}` as keyof typeof table.th;
  if (key in table.th) {
    return t(key);
  }
  return status;
}

function getWordedError(err: unknown): string {
  if (err instanceof ApiError && !isServerFailure(err)) {
    const key = `error.${err.code}` as keyof typeof table.th;
    if (Object.prototype.hasOwnProperty.call(table.th, key)) {
      return t(key);
    }
  }
  return wordedError(err);
}

function isFinished(status: string): boolean {
  return ['COMPLETED', 'CANCELLED', 'NO_SHOW', 'EXPIRED'].includes(status);
}

function canCancel(booking: MyBookingItemDto): boolean {
  if (isFinished(booking.status)) return false;
  if (!booking.cancelCutoffAt) return true;
  return new Date(booking.cancelCutoffAt).getTime() > Date.now();
}

function canReschedule(booking: MyBookingItemDto): boolean {
  if (isFinished(booking.status)) return false;
  if (!booking.cancelCutoffAt) return true;
  return new Date(booking.cancelCutoffAt).getTime() > Date.now();
}

function formatSlotTime(startIso?: string, endIso?: string): string {
  if (!startIso) return '';
  try {
    const loc = locale();
    const start = new Date(startIso);
    const end = endIso ? new Date(endIso) : null;
    const dateStr = start.toLocaleDateString(loc, {
      month: 'short',
      day: 'numeric',
      timeZone: 'Asia/Bangkok',
    });
    const startStr = start.toLocaleTimeString(loc, {
      hour: '2-digit',
      minute: '2-digit',
      timeZone: 'Asia/Bangkok',
    });
    if (!end) return `${dateStr} ${startStr}`;
    const endStr = end.toLocaleTimeString(loc, {
      hour: '2-digit',
      minute: '2-digit',
      timeZone: 'Asia/Bangkok',
    });
    return `${dateStr} ${startStr} - ${endStr}`;
  } catch {
    return `${startIso} - ${endIso ?? ''}`;
  }
}

export interface BookingsScreenProps {
  client?: ApiClient;
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
}

export function BookingsScreen(props: BookingsScreenProps = {}): JSX.Element {
  const fetchFn = props.fetch ?? ((input, init) => fetch(input, init));
  const api = props.client ?? new ApiClient(fetchFn);

  const [bookings, setBookings] = createSignal<MyBookingItemDto[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);

  // Command states
  const [commandLoading, setCommandLoading] = createSignal(false);
  const [commandError, setCommandError] = createSignal<string | null>(null);
  const [lastAction, setLastAction] = createSignal<(() => Promise<void>) | null>(null);

  // Cancel dialog
  const [cancellingBooking, setCancellingBooking] = createSignal<MyBookingItemDto | null>(null);
  const cancelWrite = keyedWrite<BookingCancelInput & { branch: string }, BookingCancelOutput>(
    api,
    'POST',
    (b) => `/api/bookings/${b.id}/cancel`,
  );

  // Reschedule dialog
  const [reschedulingBooking, setReschedulingBooking] = createSignal<MyBookingItemDto | null>(null);
  const rescheduleWrite = keyedWrite<BookingRescheduleInput & { branch: string }, BookingRescheduleOutput>(
    api,
    'POST',
    (b) => `/api/bookings/${b.id}/reschedule`,
  );
  const [availableSlots, setAvailableSlots] = createSignal<BookingSlotItemDto[]>([]);
  const [slotsLoading, setSlotsLoading] = createSignal(false);
  const [selectedSlotId, setSelectedSlotId] = createSignal<string>('');

  const fetchBookings = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const res = await api.view<MyBookingsData>('my_bookings');
      setBookings(res.data?.bookings ?? []);
    } catch (err: any) {
      setLoadError(getWordedError(err));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchBookings();
  });

  const handleOpenCancel = (booking: MyBookingItemDto) => {
    setCancellingBooking(booking);
    setCommandError(null);
  };

  const handleCloseCancel = () => {
    cancelWrite.reset();
    setCancellingBooking(null);
    setCommandError(null);
  };

  const executeCancel = async () => {
    const booking = cancellingBooking();
    if (!booking) return;
    setCommandLoading(true);
    setCommandError(null);
    setLastAction(() => executeCancel);

    try {
      const out = await cancelWrite.send({
        id: booking.id,
        reason: null,
        branch: booking.branchId,
      });
      // Update status to CANCELLED in local list
      setBookings((prev) =>
        prev.map((b) => (b.id === booking.id ? { ...b, status: out.status } : b)),
      );
      setCancellingBooking(null);
    } catch (err: any) {
      setCommandError(getWordedError(err));
    } finally {
      setCommandLoading(false);
    }
  };

  // Bumped on every slots read and on dialog close, so an answer or error
  // for a dialog that is no longer the open one is dropped (#2418).
  let slotsSeq = 0;

  const loadSlots = async (booking: MyBookingItemDto) => {
    const seq = ++slotsSeq;
    setAvailableSlots([]);
    setSlotsLoading(true);
    setCommandError(null);
    setLastAction(() => () => loadSlots(booking));

    try {
      const res = await api.view<BookingSlotsData>('booking_slots', {
        branch: booking.branchId,
      });
      if (seq !== slotsSeq) return;
      setAvailableSlots(res.data?.slots ?? []);
    } catch (err: any) {
      if (seq !== slotsSeq) return;
      setAvailableSlots([]);
      setCommandError(getWordedError(err));
    } finally {
      if (seq === slotsSeq) setSlotsLoading(false);
    }
  };

  const handleOpenReschedule = (booking: MyBookingItemDto) => {
    setReschedulingBooking(booking);
    setCommandError(null);
    setSelectedSlotId('');
    setAvailableSlots([]);
    loadSlots(booking);
  };

  const handleCloseReschedule = () => {
    slotsSeq++;
    rescheduleWrite.reset();
    setReschedulingBooking(null);
    setCommandError(null);
    setSelectedSlotId('');
    setAvailableSlots([]);
    setSlotsLoading(false);
  };

  const executeReschedule = async () => {
    const booking = reschedulingBooking();
    const slotId = selectedSlotId();
    if (!booking || !slotId) return;
    setCommandLoading(true);
    setCommandError(null);
    setLastAction(() => executeReschedule);

    try {
      await rescheduleWrite.send({
        id: booking.id,
        slotId,
        branch: booking.branchId,
      });
      // Success: re-read bookings
      await fetchBookings();
      setReschedulingBooking(null);
    } catch (err: any) {
      // On error (409 staleVersion / 400 capacityFull), re-read booking (§4.3)
      await fetchBookings();
      setCommandError(getWordedError(err));
    } finally {
      setCommandLoading(false);
    }
  };

  const handleRetryCommand = () => {
    const action = lastAction();
    if (action) {
      action();
    }
  };

  return (
    <div class="xc-bookings-screen">
      <header class="xc-bookings-header">
        <h1 class="xc-bookings-title">{t('title')}</h1>
        <p class="xc-bookings-subtitle">{t('subtitle')}</p>
      </header>

      {/* Top level command error retry banner */}
      <Show when={commandError()}>
        <div class="xc-command-error" role="alert">
          <p>{commandError()}</p>
          <button
            type="button"
            data-action="retry"
            class="xc-retry-btn"
            onClick={handleRetryCommand}
          >
            {t('retry')}
          </button>
        </div>
      </Show>

      {/* Loading state */}
      <Show when={loading()}>
        <div class="xc-bookings-loading">
          <p>{t('loading')}</p>
        </div>
      </Show>

      {/* Initial load error state */}
      <Show when={!loading() && loadError()}>
        <div class="xc-bookings-error">
          <p>{loadError()}</p>
          <button
            type="button"
            data-action="retry"
            class="xc-retry-btn"
            onClick={fetchBookings}
          >
            {t('retry')}
          </button>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!loading() && !loadError() && bookings().length === 0}>
        <div class="xc-bookings-empty">
          <p>{t('empty')}</p>
          <a href="/app/book" class="xc-btn xc-btn-primary">
            {t('bookNow')}
          </a>
        </div>
      </Show>

      {/* Booking list */}
      <Show when={!loading() && !loadError() && bookings().length > 0}>
        <div class="xc-bookings-list">
          <For each={bookings()}>
            {(booking) => {
              return (
                <div class="xc-booking-card" data-booking-id={booking.id}>
                  <div class="xc-booking-header">
                    <span class="xc-booking-no">{booking.bookingNo}</span>
                    <span class={`xc-booking-status xc-status-${booking.status.toLowerCase()}`}>
                      {getStatusLabel(booking.status)}
                    </span>
                  </div>

                  <div class="xc-booking-body">
                    <div class="xc-booking-info">
                      <p class="xc-booking-branch">{booking.branchId}</p>
                      <p class="xc-booking-time">
                        {formatSlotTime(booking.startAt, booking.endAt)}
                      </p>
                      <Show when={booking.ticketNumber}>
                        <p class="xc-booking-ticket">{booking.ticketNumber}</p>
                      </Show>
                    </div>

                    {/* Indicative quote summary */}
                    <Show when={booking.quoteSummary}>
                      <div class="xc-booking-quote">
                        <p class="xc-quote-disclaimer">{booking.quoteSummary}</p>
                      </div>
                    </Show>

                    {/* Check-in QR code */}
                    <Show when={booking.qrToken && !isFinished(booking.status)}>
                      <div class="xc-booking-qr" data-qr-token={booking.qrToken!}>
                        <QrCode value={booking.qrToken!} label={t('qrCode')} />
                      </div>
                    </Show>
                  </div>

                  {/* Actions: Cancel & Reschedule */}
                  <Show when={canCancel(booking) || canReschedule(booking)}>
                    <div class="xc-booking-actions">
                      <Show when={canCancel(booking)}>
                        <button
                          type="button"
                          class="xc-btn xc-btn-outline-danger"
                          data-action="cancel"
                          onClick={() => handleOpenCancel(booking)}
                        >
                          {t('cancel')}
                        </button>
                      </Show>
                      <Show when={canReschedule(booking)}>
                        <button
                          type="button"
                          class="xc-btn xc-btn-outline-primary"
                          data-action="reschedule"
                          onClick={() => handleOpenReschedule(booking)}
                        >
                          {t('reschedule')}
                        </button>
                      </Show>
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>

      {/* Cancel Confirmation Dialog */}
      <Show when={cancellingBooking()}>
        <div class="xc-modal-backdrop" onClick={handleCloseCancel}>
          <div
            class="xc-confirm-dialog"
            role="alertdialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            <div class="xc-dialog-content">
              <h3 class="xc-dialog-title">{t('cancel')}</h3>
              <p class="xc-dialog-prompt">{t('cancelPrompt')}</p>

              <Show when={commandError()}>
                <div class="xc-dialog-error" role="alert">
                  <p>{commandError()}</p>
                  <button
                    type="button"
                    data-action="retry"
                    class="xc-retry-btn"
                    onClick={handleRetryCommand}
                  >
                    {t('retry')}
                  </button>
                </div>
              </Show>

              <div class="xc-dialog-actions">
                <button
                  type="button"
                  class="xc-btn xc-btn-secondary"
                  onClick={handleCloseCancel}
                >
                  {t('back')}
                </button>
                <button
                  type="button"
                  class="xc-confirm-cancel-btn xc-btn xc-btn-danger"
                  data-action="confirm-cancel"
                  onClick={executeCancel}
                  disabled={commandLoading()}
                >
                  {t('confirmCancel')}
                </button>
              </div>
            </div>
          </div>
        </div>
      </Show>

      {/* Reschedule Dialog */}
      <Show when={reschedulingBooking()}>
        <div class="xc-modal-backdrop" onClick={handleCloseReschedule}>
          <div
            class="xc-reschedule-dialog"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            <div class="xc-dialog-content">
              <h3 class="xc-dialog-title">{t('selectNewSlot')}</h3>

              <Show when={commandError()}>
                <div class="xc-dialog-error" role="alert">
                  <p>{commandError()}</p>
                  <button
                    type="button"
                    data-action="retry"
                    class="xc-retry-btn"
                    onClick={handleRetryCommand}
                  >
                    {t('retry')}
                  </button>
                </div>
              </Show>

              <Show when={slotsLoading()}>
                <p class="xc-slots-loading">{t('loading')}</p>
              </Show>

              <Show when={!slotsLoading()}>
                <div class="xc-slots-list">
                  <For each={availableSlots()}>
                    {(slot) => {
                      return (
                        <div
                          class={`xc-slot-option ${selectedSlotId() === slot.id ? 'selected' : ''}`}
                          data-slot-id={slot.id}
                          onClick={() => setSelectedSlotId(slot.id)}
                        >
                          <label class="xc-slot-label">
                            <input
                              type="radio"
                              name="rescheduleSlot"
                              value={slot.id}
                              checked={selectedSlotId() === slot.id}
                              onClick={(e) => {
                                e.stopPropagation();
                                setSelectedSlotId(slot.id);
                              }}
                              onChange={() => {
                                setSelectedSlotId(slot.id);
                              }}
                            />
                            <span>{formatSlotTime(slot.start, slot.end)}</span>
                          </label>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </Show>

              <div class="xc-dialog-actions">
                <button
                  type="button"
                  class="xc-btn xc-btn-secondary"
                  onClick={handleCloseReschedule}
                >
                  {t('back')}
                </button>
                <button
                  type="button"
                  class="xc-confirm-reschedule-btn xc-btn xc-btn-primary"
                  data-action="confirm-reschedule"
                  onClick={executeReschedule}
                  disabled={commandLoading() || !selectedSlotId()}
                >
                  {t('confirmReschedule')}
                </button>
              </div>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}

export default BookingsScreen;
