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
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { QrCode } from '../../../shared/ui/QrCode';
import { formatSlotTime, getTodayInTimezone } from '../book/Book';
import { BookingExtras } from './BookingExtras';
import { BookingWait } from './BookingWait';
import * as table from './i18n';
import './bookings.css';

const t = screenT('customer.bookings', table);

function getBookingServiceDate(booking: MyBookingItemDto, timeZone?: string): string {
  const start = new Date(booking.startAt);
  if (Number.isNaN(start.getTime())) return getTodayInTimezone(timeZone as string);
  return new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(start);
}

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
  const [rescheduleDate, setRescheduleDate] = createSignal<string>('');

  const [branchNames, setBranchNames] = createSignal<Map<string, string>>(new Map());
  const [branchTimezones, setBranchTimezones] = createSignal<Map<string, string>>(new Map());
  const branchTimezone = (branchId: string) => branchTimezones().get(branchId)!;

  const loadBranches = async (): Promise<void> => {
    setBranchNames(new Map());
    setBranchTimezones(new Map());
    let data: PublicBranchesData | null = null;
    try {
      const hostRes = await fetchFn('/api/public/host');
      if (!hostRes.ok) return;
      const host = (await hostRes.json()) as { org?: string } | null;
      if (!host || typeof host.org !== 'string' || !host.org) return;

      const listRes = await fetchFn(`/api/public/orgs/${encodeURIComponent(host.org)}/branches`);
      if (!listRes.ok) return;
      data = (await listRes.json()) as PublicBranchesData | null;
    } catch {
      return;
    }

    try {
      const names = new Map<string, string>();
      const timezones = new Map<string, string>();
      for (const b of data?.branches ?? []) {
        if (!b.timezone) {
          throw new Error('missing branch timezone');
        }
        getTodayInTimezone(b.timezone);
        names.set(b.code, b.name);
        timezones.set(b.code, b.timezone);
      }
      setBranchNames(names);
      setBranchTimezones(timezones);
    } catch {
      setBranchNames(new Map());
      setBranchTimezones(new Map());
      setLoadError(t('error.loadFailed'));
    }
  };

  const fetchBookings = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const [res] = await Promise.all([
        api.view<MyBookingsData>('my_bookings'),
        loadBranches(),
      ]);
      const items = res.data?.bookings ?? [];
      setBookings(items);
      if (items.some((b) => !branchTimezone(b.branchId))) {
        setLoadError(t('error.loadFailed'));
      }
    } catch (err: any) {
      setLoadError(getWordedError(err));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchBookings();
  });

  const patchBooking = (id: string, patch: Partial<MyBookingItemDto>) => {
    setBookings((prev) => prev.map((b) => (b.id === id ? { ...b, ...patch } : b)));
  };

  let commandSeq = 0;

  const handleOpenCancel = (booking: MyBookingItemDto) => {
    commandSeq++;
    setCancellingBooking(booking);
    setCommandLoading(false);
    setCommandError(null);
    setLastAction(null);
  };

  const handleCloseCancel = () => {
    commandSeq++;
    cancelWrite.reset();
    setCancellingBooking(null);
    setCommandLoading(false);
    setCommandError(null);
    setLastAction(null);
  };

  const executeCancel = async () => {
    const booking = cancellingBooking();
    if (!booking) return;
    const bookingId = booking.id;
    const seq = ++commandSeq;
    setCommandLoading(true);
    setCommandError(null);

    try {
      const out = await cancelWrite.send({
        id: bookingId,
        reason: null,
        branch: booking.branchId,
      });
      // Update status to CANCELLED in local list
      setBookings((prev) =>
        prev.map((b) => (b.id === bookingId ? { ...b, status: out.status } : b)),
      );
      if (seq !== commandSeq || cancellingBooking()?.id !== bookingId) return;
      setCancellingBooking(null);
    } catch (err: any) {
      if (seq !== commandSeq || cancellingBooking()?.id !== bookingId) return;
      setLastAction(() => executeCancel);
      setCommandError(getWordedError(err));
    } finally {
      if (seq === commandSeq) {
        setCommandLoading(false);
      }
    }
  };

  // Bumped on every slots read and on dialog close, so an answer or error
  // for a dialog that is no longer the open one is dropped (#2418).
  let slotsSeq = 0;

  const loadSlots = async (booking: MyBookingItemDto, date: string) => {
    const seq = ++slotsSeq;
    setAvailableSlots([]);
    setSlotsLoading(true);
    setCommandError(null);
    setLastAction(() => () => loadSlots(booking, date));

    try {
      const res = await api.view<BookingSlotsData>('booking_slots', {
        branch: booking.branchId,
        filter: { date },
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
    const date = getBookingServiceDate(booking, branchTimezone(booking.branchId));
    commandSeq++;
    setReschedulingBooking(booking);
    setCommandLoading(false);
    setCommandError(null);
    setLastAction(null);
    setSelectedSlotId('');
    setRescheduleDate(date);
    setAvailableSlots([]);
    void loadSlots(booking, date);
  };

  const handleCloseReschedule = () => {
    slotsSeq++;
    commandSeq++;
    rescheduleWrite.reset();
    setReschedulingBooking(null);
    setCommandLoading(false);
    setCommandError(null);
    setLastAction(null);
    setSelectedSlotId('');
    setRescheduleDate('');
    setAvailableSlots([]);
    setSlotsLoading(false);
  };

  const executeReschedule = async () => {
    const booking = reschedulingBooking();
    const slotId = selectedSlotId();
    if (!booking || !slotId) return;
    const bookingId = booking.id;
    const seq = ++commandSeq;
    setCommandLoading(true);
    setCommandError(null);

    try {
      await rescheduleWrite.send({
        id: bookingId,
        slotId,
        branch: booking.branchId,
      });
      // Success: re-read bookings
      await fetchBookings();
      if (seq !== commandSeq || reschedulingBooking()?.id !== bookingId) return;
      setReschedulingBooking(null);
    } catch (err: any) {
      // On error (409 staleVersion / 400 capacityFull), re-read booking (§4.3)
      await fetchBookings();
      if (seq !== commandSeq || reschedulingBooking()?.id !== bookingId) return;
      setLastAction(() => executeReschedule);
      setCommandError(getWordedError(err));
    } finally {
      if (seq === commandSeq) {
        setCommandLoading(false);
      }
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
                      <p class="xc-booking-branch">{branchNames().get(booking.branchId) ?? booking.branchId}</p>
                      <p class="xc-booking-time">
                        {formatSlotTime(booking.startAt, booking.endAt, branchTimezone(booking.branchId))}
                      </p>
                      <Show when={booking.ticketNumber}>
                        <p class="xc-booking-ticket">{booking.ticketNumber}</p>
                      </Show>
                      <BookingWait booking={booking} />
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

                  <BookingExtras
                    booking={booking}
                    api={api}
                    timeZone={branchTimezone(booking.branchId)}
                    onChange={(patch) => patchBooking(booking.id, patch)}
                  />

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

              <div class="xc-reschedule-date-field" data-field="serviceDate">
                <label class="xc-reschedule-date-label" for="rescheduleServiceDate">
                  {t('date')}
                </label>
                <input
                  id="rescheduleServiceDate"
                  name="serviceDate"
                  type="date"
                  class="xc-reschedule-date-input"
                  data-testid="date-input"
                  value={rescheduleDate()}
                  min={getTodayInTimezone(branchTimezone(reschedulingBooking()!.branchId))}
                  onInput={(e) => {
                    const nextDate = e.currentTarget.value;
                    if (!nextDate || nextDate === rescheduleDate()) return;
                    const booking = reschedulingBooking();
                    if (!booking) return;
                    setSelectedSlotId('');
                    setRescheduleDate(nextDate);
                    void loadSlots(booking, nextDate);
                  }}
                  onChange={(e) => {
                    const nextDate = e.currentTarget.value;
                    if (!nextDate || nextDate === rescheduleDate()) return;
                    const booking = reschedulingBooking();
                    if (!booking) return;
                    setSelectedSlotId('');
                    setRescheduleDate(nextDate);
                    void loadSlots(booking, nextDate);
                  }}
                />
              </div>

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
                          onClick={() => {
                            if (slot.available === 0) return;
                            setSelectedSlotId(slot.id);
                          }}
                        >
                          <label class="xc-slot-label">
                            <input
                              type="radio"
                              name="rescheduleSlot"
                              value={slot.id}
                              disabled={slot.available === 0}
                              checked={selectedSlotId() === slot.id}
                              onClick={(e) => {
                                e.stopPropagation();
                                if (slot.available === 0) return;
                                setSelectedSlotId(slot.id);
                              }}
                              onChange={() => {
                                if (slot.available === 0) return;
                                setSelectedSlotId(slot.id);
                              }}
                            />
                            <span>{formatSlotTime(slot.start, slot.end, branchTimezone(reschedulingBooking()!.branchId))}</span>
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
