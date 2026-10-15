import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { DeskBookingItemDto, DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type {
  BookingCancelOutput,
  BookingCheckInOutput,
  BookingCreateOutput,
  BookingRescheduleOutput,
} from '../../../shared/api/generated/bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { en, th } from './i18n';
import './desk-bookings.css';

interface SlotGroup {
  slotId: string;
  startAt: string;
  endAt: string;
  bookings: DeskBookingItemDto[];
}

const DESK_PERMISSION = 'BookingsDesk';
const VIEW_DESK_BOOKINGS = 'desk_bookings';
const VIEW_STAFF_ME = 'staff_me';
const ROUTE_BOOKINGS = '/api/bookings';
const routeCheckIn = (id: string) => `/api/bookings/${encodeURIComponent(id)}/check-in`;
const routeCancel = (id: string) => `/api/bookings/${encodeURIComponent(id)}/cancel`;
const routeReschedule = (id: string) => `/api/bookings/${encodeURIComponent(id)}/reschedule`;

const t = screenT('desk.bookings', { th, en });

function deskBookingError(err: unknown): string {
  if (err instanceof ApiError && !isServerFailure(err)) {
    const key = `error.${err.code}` as keyof typeof th;
    if (Object.prototype.hasOwnProperty.call(th, key)) {
      return t(key);
    }
  }
  return wordedError(err);
}

function formatSlotTime(startAt?: string, endAt?: string, timeZone = 'Asia/Bangkok'): string {
  if (!startAt) return '';
  try {
    const fmt = new Intl.DateTimeFormat('en-GB', {
      timeZone,
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
    const s = fmt.format(new Date(startAt));
    const e = endAt ? fmt.format(new Date(endAt)) : '';
    return e ? `${s} - ${e}` : s;
  } catch {
    return endAt ? `${startAt} - ${endAt}` : startAt;
  }
}

function groupBookingsBySlot(items: DeskBookingItemDto[]): SlotGroup[] {
  const map = new Map<string, SlotGroup>();
  for (const item of items) {
    let group = map.get(item.slotId);
    if (!group) {
      group = {
        slotId: item.slotId,
        startAt: item.startAt,
        endAt: item.endAt,
        bookings: [],
      };
      map.set(item.slotId, group);
    }
    group.bookings.push(item);
  }
  return Array.from(map.values());
}

export function DeskBookingsScreen(): JSX.Element {
  const api = new ApiClient();
  const [authorized, setAuthorized] = createSignal<boolean | null>(null);
  const [refusalMessage, setRefusalMessage] = createSignal<string | null>(null);
  const [items, setItems] = createSignal<DeskBookingItemDto[]>([]);
  const [branchId, setBranchId] = createSignal<string>('');
  const [version, setVersion] = createSignal<number>(1);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [retryAction, setRetryAction] = createSignal<(() => Promise<void>) | null>(null);

  // QR search state
  const [qrInput, setQrInput] = createSignal('');
  const [activeQr, setActiveQr] = createSignal('');

  // Cancel dialog state
  const [cancellingBookingId, setCancellingBookingId] = createSignal<string | null>(null);
  const [cancelReason, setCancelReason] = createSignal('');

  // Reschedule dialog state
  const [reschedulingBookingId, setReschedulingBookingId] = createSignal<string | null>(null);
  const [targetSlotId, setTargetSlotId] = createSignal('');

  // New booking modal state
  const [showNewBookingModal, setShowNewBookingModal] = createSignal(false);
  const [formBranchId, setFormBranchId] = createSignal('');
  const [formSlotId, setFormSlotId] = createSignal('');
  const [formShopSide, setFormShopSide] = createSignal('SELL');
  const [formCurrency, setFormCurrency] = createSignal('USD');
  const [formQuantity, setFormQuantity] = createSignal('');
  const [formDisplayName, setFormDisplayName] = createSignal('');
  const [formPhone, setFormPhone] = createSignal('');
  const [formReferralCode, setFormReferralCode] = createSignal('');

  const allSlots = () => groupBookingsBySlot(items());

  async function fetchBookings(opts?: { since?: number; qr?: string }) {
    const qrParam = opts?.qr !== undefined ? opts.qr : activeQr();
    try {
      const envelope = await api.view<DeskBookingsData>(VIEW_DESK_BOOKINGS, {
        since: opts?.since,
        filter: qrParam ? { qr: qrParam } : undefined,
      });
      if (envelope.version !== undefined) {
        setVersion(envelope.version);
      }
      if (envelope.changed === false) {
        return;
      }
      if (envelope.data && Array.isArray(envelope.data.items)) {
        setItems(envelope.data.items);
        const groups = groupBookingsBySlot(envelope.data.items);
        const firstSlot = groups[0]?.slotId;
        if (firstSlot && !formSlotId()) {
          setFormSlotId(firstSlot);
        }
        setAuthorized(true);
        setRefusalMessage(null);
      }
    } catch (err) {
      if (err instanceof ApiError && err.code === 'roleNotAllowed') {
        setAuthorized(false);
        setRefusalMessage(t('error.roleNotAllowed'));
        return;
      }
      setErrorMessage(deskBookingError(err));
    }
  }

  onMount(async () => {
    try {
      const staffEnv = await api.view<StaffMeView>(VIEW_STAFF_ME);
      const perms: string[] = staffEnv.data?.permissions ?? [];
      if (!perms.includes(DESK_PERMISSION)) {
        setAuthorized(false);
        setRefusalMessage(t('error.roleNotAllowed'));
        return;
      }
      if (staffEnv.data?.branchId) {
        setBranchId(staffEnv.data.branchId);
        setFormBranchId(staffEnv.data.branchId);
      }
    } catch {
      // ignore
    }

    await fetchBookings();
  });

  async function handleRefresh() {
    await fetchBookings({ since: version() });
  }

  async function handleQrLookup() {
    const q = qrInput().trim();
    setActiveQr(q);
    await fetchBookings({ qr: q });
  }

  function handleQrKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      handleQrLookup();
    }
  }

  function handleRetry() {
    const act = retryAction();
    if (act) {
      act();
    }
  }

  async function handleCheckIn(bookingId: string, existingKey?: string) {
    const key = existingKey ?? crypto.randomUUID();
    setErrorMessage(null);
    setRetryAction(null);

    try {
      const resData = await api.command<{ identityChecked: boolean; branch: string }, BookingCheckInOutput>(
        'POST',
        routeCheckIn(bookingId),
        { identityChecked: true, branch: branchId() },
        key,
      );
      if (!resData || typeof resData.status !== 'string') {
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        status: resData.status,
        ticketNumber: resData.ticketNumber,
      });
    } catch (err) {
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleCheckIn(bookingId, key));
    }
  }

  function openCancel(bookingId: string) {
    setCancellingBookingId(bookingId);
    setCancelReason('');
    setErrorMessage(null);
    setRetryAction(null);
  }

  async function handleConfirmCancel(existingKey?: string) {
    const bookingId = cancellingBookingId();
    if (!bookingId) return;
    const reason = cancelReason().trim();
    if (!reason) {
      return;
    }

    const key = existingKey ?? crypto.randomUUID();
    setErrorMessage(null);
    setRetryAction(null);

    try {
      const resData = await api.command<{ reason: string; branch: string }, BookingCancelOutput>(
        'POST',
        routeCancel(bookingId),
        { reason, branch: branchId() },
        key,
      );
      if (!resData || typeof resData.status !== 'string') {
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        status: resData.status,
      });
      setCancellingBookingId(null);
      setCancelReason('');
    } catch (err) {
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleConfirmCancel(key));
    }
  }

  function openReschedule(bookingId: string) {
    setReschedulingBookingId(bookingId);
    setErrorMessage(null);
    setRetryAction(null);
    const booking = items().find((b) => b.id === bookingId);
    const slots = allSlots();
    const otherSlot = slots.find((s) => s.slotId !== booking?.slotId) ?? slots[0];
    setTargetSlotId(otherSlot?.slotId ?? '');
  }

  async function handleConfirmReschedule(existingKey?: string) {
    const bookingId = reschedulingBookingId();
    if (!bookingId) return;
    const target = targetSlotId();
    if (!target) return;

    const key = existingKey ?? crypto.randomUUID();
    setErrorMessage(null);
    setRetryAction(null);

    try {
      const resData = await api.command<{ slotId: string; branch: string }, BookingRescheduleOutput>(
        'POST',
        routeReschedule(bookingId),
        { slotId: target, branch: branchId() },
        key,
      );
      if (!resData || typeof resData.status !== 'string' || typeof resData.slotId !== 'string') {
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        slotId: resData.slotId,
        status: resData.status,
      });
      setReschedulingBookingId(null);
    } catch (err) {
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleConfirmReschedule(key));
    }
  }

  function openNewBooking() {
    setErrorMessage(null);
    setRetryAction(null);
    setShowNewBookingModal(true);
    const slots = allSlots();
    if (!formSlotId() && slots.length > 0) {
      setFormSlotId(slots[0].slotId);
    }
  }

  function resetNewBookingForm() {
    setFormQuantity('');
    setFormDisplayName('');
    setFormPhone('');
    setFormReferralCode('');
  }

  let isSubmittingBooking = false;
  async function handleSubmitNewBooking(e?: Event) {
    if (e) {
      e.preventDefault();
    }
    if (isSubmittingBooking) return;
    isSubmittingBooking = true;
    try {
      await handleCreateBooking();
    } finally {
      isSubmittingBooking = false;
    }
  }

  async function handleCreateBooking(existingKey?: string) {
    const key = existingKey ?? crypto.randomUUID();
    setErrorMessage(null);
    setRetryAction(null);

    const qty = Number(formQuantity());
    const targetBranch = formBranchId() || branchId();
    const payload: Record<string, unknown> = {
      branch: targetBranch,
      branchId: targetBranch,
      slotId: formSlotId(),
      shopSide: formShopSide(),
      currency: formCurrency(),
      quantity: isNaN(qty) ? 0 : qty,
      displayName: formDisplayName(),
      phone: formPhone(),
    };
    const ref = formReferralCode().trim();
    if (ref) {
      payload.referralCode = ref;
    }

    try {
      await api.command<Record<string, unknown>, BookingCreateOutput>(
        'POST',
        ROUTE_BOOKINGS,
        payload,
        key,
      );
      setShowNewBookingModal(false);
      resetNewBookingForm();
      await fetchBookings();
    } catch (err) {
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleCreateBooking(key));
    }
  }

  function updateBookingInState(bookingId: string, patch: Partial<DeskBookingItemDto>) {
    setItems((prev) => {
      let targetStart: string | undefined;
      let targetEnd: string | undefined;
      if (patch.slotId) {
        const peer = prev.find((b) => b.slotId === patch.slotId);
        if (peer) {
          targetStart = peer.startAt;
          targetEnd = peer.endAt;
        }
      }
      return prev.map((b) => {
        if (b.id !== bookingId) return b;
        return {
          ...b,
          ...patch,
          startAt: targetStart ?? b.startAt,
          endAt: targetEnd ?? b.endAt,
        };
      });
    });
  }

  return (
    <>
      <Show when={authorized() === false}>
        <div role="alert" class="xc-refusal">
          {refusalMessage() ?? th['error.roleNotAllowed']}
        </div>
      </Show>

      <Show when={authorized() === true}>
        <div class="xc-desk-bookings" data-screen="desk-bookings">
          <Show when={errorMessage()}>
            <div role="alert" class="xc-error-banner">
              <span>{errorMessage()}</span>
              <Show when={retryAction()}>
                <button
                  type="button"
                  data-action="retry"
                  class="xc-retry-btn xc-btn-secondary"
                  onClick={handleRetry}
                >
                  {th.retry}
                </button>
              </Show>
            </div>
          </Show>

          <div class="xc-bookings-toolbar">
            <div class="xc-qr-search">
              <input
                name="qrToken"
                data-action="scan-qr"
                class="xc-qr-input"
                placeholder={th.scanQr}
                value={qrInput()}
                onInput={(e) => setQrInput(e.currentTarget.value)}
                onKeyDown={handleQrKeyDown}
              />
              <button
                type="button"
                data-action="lookup-qr"
                class="xc-qr-submit xc-btn-secondary"
                onClick={handleQrLookup}
              >
                {th.scanQr}
              </button>
            </div>

            <div class="xc-bookings-actions">
              <button
                type="button"
                data-action="refresh"
                class="xc-bookings-refresh xc-btn-secondary"
                onClick={handleRefresh}
              >
                Refresh
              </button>
              <button
                type="button"
                data-action="new-booking"
                class="xc-new-booking-btn xc-btn-primary"
                onClick={openNewBooking}
              >
                {th.newBooking}
              </button>
            </div>
          </div>

          <div class="xc-slots-container">
            <Show
              when={allSlots().length > 0}
              fallback={<div class="xc-empty-state">{th.empty}</div>}
            >
              <For each={allSlots()}>
                {(slot) => (
                  <div class="xc-slot-group">
                    <div class="xc-slot-header">
                      <span>
                        {formatSlotTime(slot.startAt, slot.endAt) || slot.slotId}
                      </span>
                    </div>

                    <div class="xc-slot-bookings">
                      <For each={slot.bookings}>
                        {(b) => (
                          <div class="xc-booking-item" data-booking-id={b.id}>
                            <div class="xc-booking-info">
                              <span class="xc-booking-no">{b.bookingNo}</span>
                              <span class="xc-customer-name">{b.customerDisplayName}</span>
                              <span class="xc-phone-last4">{b.phoneLast4}</span>
                              <span class="xc-readiness">{b.readiness}</span>
                              <Show when={b.ticketNumber}>
                                <span class="xc-ticket-number">{b.ticketNumber}</span>
                              </Show>
                              <span class={`xc-status-tag xc-status-${b.status}`}>
                                {th[`status.${b.status}` as keyof typeof th] ?? th.statusUnknown}
                              </span>
                            </div>

                            <div class="xc-booking-actions">
                              <button
                                type="button"
                                data-action="check-in"
                                class="xc-checkin-btn xc-btn-primary"
                                onClick={() => handleCheckIn(b.id)}
                              >
                                {th.checkIn}
                              </button>
                              <button
                                type="button"
                                data-action="cancel"
                                class="xc-cancel-btn xc-btn-secondary"
                                onClick={() => openCancel(b.id)}
                              >
                                {th.cancel}
                              </button>
                              <button
                                type="button"
                                data-action="reschedule"
                                class="xc-reschedule-btn xc-btn-secondary"
                                onClick={() => openReschedule(b.id)}
                              >
                                {th.reschedule}
                              </button>
                            </div>
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                )}
              </For>
            </Show>
          </div>

          {/* Cancellation Dialog */}
          <Show when={cancellingBookingId()}>
            <div class="xc-modal-overlay">
              <div class="xc-modal">
                <h3 class="xc-modal-title">{th.cancel}</h3>
                <div class="xc-form-group">
                  <label for="cancel-reason-input">{th.reasonRequired}</label>
                  <input
                    id="cancel-reason-input"
                    name="reason"
                    class="xc-cancel-reason"
                    value={cancelReason()}
                    onInput={(e) => setCancelReason(e.currentTarget.value)}
                    placeholder={th.reasonRequired}
                  />
                </div>
                <div class="xc-modal-actions">
                  <button
                    type="button"
                    class="xc-btn-secondary"
                    onClick={() => setCancellingBookingId(null)}
                  >
                    Close
                  </button>
                  <button
                    type="button"
                    data-action="confirm-cancel"
                    class="xc-confirm-cancel xc-btn-primary"
                    onClick={() => handleConfirmCancel()}
                  >
                    {th.cancel}
                  </button>
                </div>
              </div>
            </div>
          </Show>

          {/* Reschedule Dialog */}
          <Show when={reschedulingBookingId()}>
            <div class="xc-modal-overlay">
              <div class="xc-modal">
                <h3 class="xc-modal-title">{th.reschedule}</h3>
                <div class="xc-form-group">
                  <label for="reschedule-slot-select">Slot</label>
                  <select
                    id="reschedule-slot-select"
                    name="targetSlotId"
                    value={targetSlotId()}
                    onInput={(e) => setTargetSlotId(e.currentTarget.value)}
                    onChange={(e) => setTargetSlotId(e.currentTarget.value)}
                  >
                    <For each={allSlots()}>
                      {(s) => (
                        <option value={s.slotId}>
                          {formatSlotTime(s.startAt, s.endAt) || s.slotId}
                        </option>
                      )}
                    </For>
                    <Show when={targetSlotId() && !allSlots().some((s) => s.slotId === targetSlotId())}>
                      <option value={targetSlotId()}>{targetSlotId()}</option>
                    </Show>
                  </select>
                </div>
                <div class="xc-modal-actions">
                  <button
                    type="button"
                    class="xc-btn-secondary"
                    onClick={() => setReschedulingBookingId(null)}
                  >
                    Close
                  </button>
                  <button
                    type="button"
                    data-action="confirm-reschedule"
                    class="xc-confirm-reschedule xc-btn-primary"
                    onClick={() => handleConfirmReschedule()}
                  >
                    {th.reschedule}
                  </button>
                </div>
              </div>
            </div>
          </Show>

          {/* New Booking Modal */}
          <Show when={showNewBookingModal()}>
            <div class="xc-modal-overlay">
              <div class="xc-modal">
                <h3 class="xc-modal-title">{th.newBooking}</h3>
                <form onSubmit={handleSubmitNewBooking}>
                  <div class="xc-form-group">
                    <label for="new-booking-branch">Branch</label>
                    <input
                      id="new-booking-branch"
                      name="branchId"
                      value={formBranchId()}
                      onInput={(e) => setFormBranchId(e.currentTarget.value)}
                      onChange={(e) => setFormBranchId(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-slot">Slot</label>
                    <select
                      id="new-booking-slot"
                      name="slotId"
                      value={formSlotId()}
                      onInput={(e) => setFormSlotId(e.currentTarget.value)}
                      onChange={(e) => setFormSlotId(e.currentTarget.value)}
                    >
                      <For each={allSlots()}>
                        {(s) => (
                          <option value={s.slotId}>
                            {formatSlotTime(s.startAt, s.endAt) || s.slotId}
                          </option>
                        )}
                      </For>
                      <Show when={formSlotId() && !allSlots().some((s) => s.slotId === formSlotId())}>
                        <option value={formSlotId()}>{formSlotId()}</option>
                      </Show>
                    </select>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-side">Side</label>
                    <select
                      id="new-booking-side"
                      name="shopSide"
                      value={formShopSide()}
                      onInput={(e) => setFormShopSide(e.currentTarget.value)}
                      onChange={(e) => setFormShopSide(e.currentTarget.value)}
                    >
                      <option value="BUY">BUY</option>
                      <option value="SELL">SELL</option>
                    </select>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-currency">Currency</label>
                    <input
                      id="new-booking-currency"
                      name="currency"
                      value={formCurrency()}
                      onInput={(e) => setFormCurrency(e.currentTarget.value)}
                      onChange={(e) => setFormCurrency(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-quantity">Quantity</label>
                    <input
                      id="new-booking-quantity"
                      type="number"
                      name="quantity"
                      value={formQuantity()}
                      onInput={(e) => setFormQuantity(e.currentTarget.value)}
                      onChange={(e) => setFormQuantity(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-display-name">Customer Name</label>
                    <input
                      id="new-booking-display-name"
                      name="displayName"
                      value={formDisplayName()}
                      onInput={(e) => setFormDisplayName(e.currentTarget.value)}
                      onChange={(e) => setFormDisplayName(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-phone">Customer Phone</label>
                    <input
                      id="new-booking-phone"
                      type="tel"
                      name="phone"
                      value={formPhone()}
                      onInput={(e) => setFormPhone(e.currentTarget.value)}
                      onChange={(e) => setFormPhone(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-referral">Referral Code</label>
                    <input
                      id="new-booking-referral"
                      name="referralCode"
                      class="xc-referral-input"
                      value={formReferralCode()}
                      onInput={(e) => setFormReferralCode(e.currentTarget.value)}
                      onChange={(e) => setFormReferralCode(e.currentTarget.value)}
                    />
                  </div>

                  <div class="xc-modal-actions">
                    <button
                      type="button"
                      class="xc-btn-secondary"
                      onClick={() => setShowNewBookingModal(false)}
                    >
                      Close
                    </button>
                    <button
                      type="submit"
                      data-action="submit-booking"
                      class="xc-submit-booking xc-btn-primary"
                      onClick={handleSubmitNewBooking}
                    >
                      {th.newBooking}
                    </button>
                  </div>
                </form>
              </div>
            </div>
          </Show>
        </div>
      </Show>
    </>
  );
}
