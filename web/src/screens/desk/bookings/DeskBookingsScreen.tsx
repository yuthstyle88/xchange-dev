import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { DeskBookingItemDto, DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { DeskConfig } from '../../../shared/api/generated/desk_config';
import type {
  BookingCancelOutput,
  BookingCheckInOutput,
  BookingCreateOutput,
  BookingQrReissueOutput,
  BookingRescheduleOutput,
} from '../../../shared/api/generated/bookings';
import type { MeView } from '../../../shared/api/generated/me';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { parseRequestQr } from '../../../shared/queue/request-qr';
import { QrCode } from '../../../shared/ui/QrCode';
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
const VIEW_ME = 'me';
const ROUTE_BOOKINGS = '/api/bookings';
const routeCheckIn = (id: string) => `/api/bookings/${encodeURIComponent(id)}/check-in`;
const routeCancel = (id: string) => `/api/bookings/${encodeURIComponent(id)}/cancel`;
const routeReschedule = (id: string) => `/api/bookings/${encodeURIComponent(id)}/reschedule`;
const routeQr = (id: string) => `/api/bookings/${encodeURIComponent(id)}/qr`;

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
  const [version, setVersion] = createSignal<number | undefined>(undefined);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [retryAction, setRetryAction] = createSignal<(() => Promise<void>) | null>(null);

  // QR search state
  const [qrInput, setQrInput] = createSignal('');
  const [activeQr, setActiveQr] = createSignal('');

  // Cancel dialog state
  const [cancellingBookingId, setCancellingBookingId] = createSignal<string | null>(null);
  const [cancelReason, setCancelReason] = createSignal('');
  const [cancelReasonError, setCancelReasonError] = createSignal<string | null>(null);

  // Reschedule dialog state
  const [reschedulingBookingId, setReschedulingBookingId] = createSignal<string | null>(null);
  const [targetSlotId, setTargetSlotId] = createSignal('');

  // QR reissue dialog state
  const [qrDialogData, setQrDialogData] = createSignal<{ bookingNo: string; qrToken: string | null } | null>(null);

  // New booking modal state
  const [showNewBookingModal, setShowNewBookingModal] = createSignal(false);
  const [currencies, setCurrencies] = createSignal<string[]>(['USD']);
  const [formBranchId, setFormBranchId] = createSignal('');
  const [formSlotId, setFormSlotId] = createSignal('');
  const [formShopSide, setFormShopSide] = createSignal('SELL');
  const [formCurrency, setFormCurrency] = createSignal('USD');
  const [formQuantity, setFormQuantity] = createSignal('');
  const [quantityInvalid, setQuantityInvalid] = createSignal(false);
  const [formDisplayName, setFormDisplayName] = createSignal('');
  const [formPhone, setFormPhone] = createSignal('');
  const [formReferralCode, setFormReferralCode] = createSignal('');
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  function clearFieldError(field: string) {
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
  }

  const allSlots = () => groupBookingsBySlot(items());
  let readErrorActive = false;
  let readSeq = 0;
  let settledSeq = 0;
  let loadedQr: string | null = null;

  async function fetchBookings(opts?: { since?: number; qr?: string }) {
    const seq = ++readSeq;
    const qrParam = opts?.qr !== undefined ? opts.qr : activeQr();
    const sinceParam = loadedQr === qrParam ? opts?.since : undefined;
    const branchParam = branchId().trim();
    try {
      const envelope = await api.view<DeskBookingsData>(VIEW_DESK_BOOKINGS, {
        branch: branchParam ? branchParam : undefined,
        since: sinceParam,
        filter: qrParam ? { qr: qrParam } : undefined,
      });
      if (seq < settledSeq) return;
      if (envelope.changed === false) {
        if (loadedQr !== qrParam || seq !== readSeq) {
          return;
        }
        settledSeq = seq;
        if (envelope.version !== undefined) {
          setVersion(envelope.version);
        }
        if (readErrorActive) {
          readErrorActive = false;
          setErrorMessage(null);
          setRetryAction(null);
        }
        return;
      }
      if (seq !== readSeq && loadedQr === qrParam) return;
      if (qrParam !== activeQr()) return;
      settledSeq = Math.max(settledSeq, seq);
      if (envelope.version !== undefined) {
        setVersion(envelope.version);
      }
      if (readErrorActive) {
        readErrorActive = false;
        setErrorMessage(null);
        setRetryAction(null);
      }
      if (envelope.data && Array.isArray(envelope.data.items)) {
        loadedQr = qrParam;
        const mappedItems: DeskBookingItemDto[] = envelope.data.items;
        setItems(mappedItems);
        const groups = groupBookingsBySlot(mappedItems);
        const firstSlot = groups[0]?.slotId;
        if (firstSlot && !formSlotId()) {
          setFormSlotId(firstSlot);
        }
        setAuthorized(true);
        setRefusalMessage(null);
      }
    } catch (err) {
      if (seq !== readSeq || seq < settledSeq) return;
      settledSeq = seq;
      if (err instanceof ApiError && err.code === 'roleNotAllowed') {
        setAuthorized(false);
        setRefusalMessage(t('error.roleNotAllowed'));
        return;
      }
      readErrorActive = true;
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => fetchBookings(opts));
    }
  }

  onMount(async () => {
    try {
      const meEnv = await api.view<MeView>(VIEW_ME);
      const perms: string[] = meEnv.data?.permissions ?? [];
      if (!perms.includes(DESK_PERMISSION)) {
        setAuthorized(false);
        setRefusalMessage(t('error.roleNotAllowed'));
        return;
      }
      const urlBranch =
        typeof window !== 'undefined'
          ? new URLSearchParams(window.location.search).get('branch')?.trim() ?? ''
          : '';
      const roleBranch =
        meEnv.data?.roles
          ?.find((r) => typeof r.branchId === 'string' && r.branchId.trim() !== '')
          ?.branchId?.trim() ?? '';
      const resolvedBranch = urlBranch || roleBranch;
      if (resolvedBranch) {
        setBranchId(resolvedBranch);
        setFormBranchId(resolvedBranch);
      }
      void api
        .view<DeskConfig>('desk_config', resolvedBranch ? { branch: resolvedBranch } : undefined)
        .then((cfgEnv) => {
          const codes = (cfgEnv.data?.rates ?? []).map((r) => r.code).filter(Boolean);
          if (codes.length > 0) {
            setCurrencies(codes);
            if (!codes.includes(formCurrency())) {
              setFormCurrency(codes[0]!);
            }
          }
        })
        .catch(() => {
          // ignore
        });
    } catch {
      // ignore
    }

    await fetchBookings();
  });

  async function handleRefresh() {
    const q = activeQr();
    const since = loadedQr === q ? version() : undefined;
    await fetchBookings({ since });
  }

  async function handleQrLookup() {
    const raw = qrInput();
    const requestToken = parseRequestQr(raw);
    if (requestToken !== null) {
      readErrorActive = false;
      setErrorMessage(null);
      setRetryAction(null);
      const branch = branchId().trim();
      try {
        const res = await api.post<
          { action: 'findByToken'; payload: { branchId: string; token: string } },
          { requestId: string }
        >('/api/kiosk', {
          action: 'findByToken',
          payload: { branchId: branch, token: requestToken },
        });
        window.location.assign(
          `/desk?branch=${encodeURIComponent(branch)}&kiosk=${encodeURIComponent(res.requestId)}`,
        );
      } catch (err) {
        readErrorActive = false;
        // QW-1 (§4.3a W8): one 404 for another branch, an unknown and an expired token.
        setErrorMessage(
          err instanceof ApiError && err.code === 'notFound'
            ? t('requestQrNotFound')
            : deskBookingError(err),
        );
      }
      return;
    }
    const q = raw.trim();
    if (q !== loadedQr) {
      setVersion(undefined);
    }
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

  let activeCheckInId = '';
  const checkInWrite = keyedWrite<{ identityChecked: boolean; branch: string }, BookingCheckInOutput>(
    api,
    'POST',
    () => routeCheckIn(activeCheckInId),
  );
  let activeCancelId = '';
  const cancelWrite = keyedWrite<{ reason: string; branch: string }, BookingCancelOutput>(
    api,
    'POST',
    // #2464: the path is resolved synchronously inside send(), right after
    // submitCancel sets activeCancelId to the booking the write is for, so a
    // dialog opened for another booking meanwhile cannot redirect it.
    () => routeCancel(activeCancelId),
  );
  let activeRescheduleId = '';
  const rescheduleWrite = keyedWrite<{ slotId: string; branch: string }, BookingRescheduleOutput>(
    api,
    'POST',
    () => routeReschedule(activeRescheduleId),
  );
  let activeQrId = '';
  const qrWrite = keyedWrite<{ id: string; branch: string }, BookingQrReissueOutput>(
    api,
    'POST',
    () => routeQr(activeQrId),
  );
  const createWrite = keyedWrite<Record<string, unknown>, BookingCreateOutput>(
    api,
    'POST',
    ROUTE_BOOKINGS,
  );

  // #2411: only one booking write (check-in / cancel / reschedule) at a time.
  // A second send while the first is in flight reuses the idempotency key, the
  // server answers 409 idempotencyConflict, and keyedWrite then drops the key.
  const [writing, setWriting] = createSignal(false);

  async function handleCheckIn(bookingId: string) {
    if (writing()) return;
    if (activeCheckInId !== bookingId) {
      checkInWrite.reset();
      activeCheckInId = bookingId;
    }
    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);

    setWriting(true);
    try {
      const resData = await checkInWrite.send({
        identityChecked: true,
        branch: branchId(),
      });
      if (!resData || typeof resData.status !== 'string') {
        readErrorActive = false;
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        status: resData.status,
        ticketNumber: resData.ticketNumber,
      });
    } catch (err) {
      readErrorActive = false;
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleCheckIn(bookingId));
    } finally {
      setWriting(false);
    }
  }

  async function handleReissueQr(bookingId: string, bookingNo: string) {
    if (writing()) return;
    if (activeQrId !== bookingId) {
      qrWrite.reset();
      activeQrId = bookingId;
    }
    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);

    setWriting(true);
    try {
      const resData = await qrWrite.send({
        id: bookingId,
        branch: branchId(),
      });
      if (!resData) {
        readErrorActive = false;
        setErrorMessage(t('error.validationFailed'));
        return;
      }
      setQrDialogData({
        bookingNo,
        qrToken: resData.qrToken ?? null,
      });
      await fetchBookings();
    } catch (err) {
      readErrorActive = false;
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => handleReissueQr(bookingId, bookingNo));
    } finally {
      setWriting(false);
    }
  }

  function openCancel(bookingId: string) {
    // #2475: while a write is in flight or its Retry is offered, keep its key so
    // that Retry replays it; submitCancel still resets when another booking is sent.
    if (!writing() && !retryAction() && activeCancelId !== bookingId) {
      cancelWrite.reset();
      activeCancelId = bookingId;
    }
    setCancellingBookingId(bookingId);
    setCancelReason('');
    setCancelReasonError(null);
    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);
  }

  async function handleConfirmCancel() {
    if (writing()) return;
    const bookingId = cancellingBookingId();
    if (!bookingId) return;
    const reason = cancelReason().trim();
    if (!reason || reason.length > 200) {
      readErrorActive = false;
      const msg = !reason ? t('reasonRequired') : t('error.validationFailed');
      setCancelReasonError(msg);
      setErrorMessage(msg);
      setRetryAction(null);
      return;
    }
    await submitCancel(bookingId, reason);
  }

  // #2464: the write, its late answer and its Retry are all bound to the
  // booking it was sent for, not to whichever dialog is open when it answers.
  async function submitCancel(bookingId: string, reason: string) {
    if (writing()) return;
    if (activeCancelId !== bookingId) {
      cancelWrite.reset();
      activeCancelId = bookingId;
    }

    readErrorActive = false;
    setCancelReasonError(null);
    setErrorMessage(null);
    setRetryAction(null);

    setWriting(true);
    try {
      const resData = await cancelWrite.send({
        reason,
        branch: branchId(),
      });
      if (!resData || typeof resData.status !== 'string') {
        readErrorActive = false;
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        status: resData.status,
      });
      if (cancellingBookingId() === bookingId) {
        setCancellingBookingId(null);
        setCancelReason('');
        setCancelReasonError(null);
      }
    } catch (err) {
      readErrorActive = false;
      const fe = fieldError(err, { reason: t('reasonLabel') });
      if (fe.field === 'reason') {
        setCancelReasonError(fe.message);
        setErrorMessage(null);
        setRetryAction(null);
      } else {
        setCancelReasonError(null);
        setErrorMessage(deskBookingError(err));
        setRetryAction(() => () => submitCancel(bookingId, reason));
      }
    } finally {
      setWriting(false);
    }
  }

  function openReschedule(bookingId: string) {
    // #2475: see openCancel.
    if (!writing() && !retryAction() && activeRescheduleId !== bookingId) {
      rescheduleWrite.reset();
      activeRescheduleId = bookingId;
    }
    setReschedulingBookingId(bookingId);
    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);
    const booking = items().find((b) => b.id === bookingId);
    const slots = allSlots();
    const otherSlot = slots.find((s) => s.slotId !== booking?.slotId) ?? slots[0];
    setTargetSlotId(otherSlot?.slotId ?? '');
  }

  async function handleConfirmReschedule() {
    if (writing()) return;
    const bookingId = reschedulingBookingId();
    if (!bookingId) return;
    const target = targetSlotId();
    if (!target) return;
    await submitReschedule(bookingId, target);
  }

  async function submitReschedule(bookingId: string, target: string) {
    if (writing()) return;
    if (activeRescheduleId !== bookingId) {
      rescheduleWrite.reset();
      activeRescheduleId = bookingId;
    }

    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);

    setWriting(true);
    try {
      const resData = await rescheduleWrite.send({
        slotId: target,
        branch: branchId(),
      });
      if (!resData || typeof resData.status !== 'string' || typeof resData.slotId !== 'string') {
        readErrorActive = false;
        setErrorMessage(t('error.validationFailed'));
        return;
      }

      updateBookingInState(bookingId, {
        slotId: resData.slotId,
        status: resData.status,
      });
      if (reschedulingBookingId() === bookingId) {
        setReschedulingBookingId(null);
      }
    } catch (err) {
      readErrorActive = false;
      setErrorMessage(deskBookingError(err));
      setRetryAction(() => () => submitReschedule(bookingId, target));
    } finally {
      setWriting(false);
    }
  }

  function openNewBooking() {
    readErrorActive = false;
    setErrorMessage(null);
    setRetryAction(null);
    setQuantityInvalid(false);
    resetNewBookingForm();
    setShowNewBookingModal(true);
    const slots = allSlots();
    if (!formSlotId() && slots.length > 0) {
      setFormSlotId(slots[0].slotId);
    }
  }

  function resetNewBookingForm() {
    setFormQuantity('');
    setQuantityInvalid(false);
    setFormDisplayName('');
    setFormPhone('');
    setFormReferralCode('');
    setFieldErrors({});
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

  async function handleCreateBooking() {
    if (writing()) return;

    setFieldErrors({});

    const rawQty = formQuantity().trim();
    const qty = Number(rawQty);
    if (!/^\d+$/.test(rawQty) || !Number.isSafeInteger(qty) || qty <= 0) {
      setQuantityInvalid(true);
      readErrorActive = false;
      setErrorMessage(null);
      setRetryAction(null);
      return;
    }
    setQuantityInvalid(false);

    const phone = formPhone().trim();
    const ref = formReferralCode().trim();
    const nextErrors: Record<string, string> = {};
    if (!phone || !/^[+0][0-9 -]{8,}$/.test(phone)) {
      nextErrors.phone = t('error.invalidPhone');
    }
    if (ref && !/^[A-Za-z0-9]{6,12}$/.test(ref)) {
      nextErrors.referralCode = t('error.invalidReferralCode');
    }
    if (Object.keys(nextErrors).length > 0) {
      setFieldErrors(nextErrors);
      readErrorActive = false;
      setErrorMessage(null);
      setRetryAction(null);
      return;
    }

    const targetBranch = formBranchId() || branchId();
    const payload: Record<string, unknown> = {
      branch: targetBranch,
      branchId: targetBranch,
      slotId: formSlotId(),
      shopSide: formShopSide(),
      currency: formCurrency(),
      quantity: qty,
      displayName: formDisplayName(),
    };
    if (phone) {
      payload.phone = phone;
    }
    if (ref) {
      payload.referralCode = ref;
    }

    setWriting(true);
    try {
      await createWrite.send(payload);
      readErrorActive = false;
      setErrorMessage(null);
      setRetryAction(null);
      setShowNewBookingModal(false);
      resetNewBookingForm();
      await fetchBookings();
    } catch (err) {
      readErrorActive = false;
      const fe = fieldError(err, {
        currency: t('field.currency'),
        phone: t('field.phone'),
        referralCode: t('field.referralCode'),
        quantity: t('field.quantity'),
        slotId: t('field.slotId'),
        displayName: t('field.displayName'),
      });
      if (fe.field) {
        setErrorMessage(null);
        setRetryAction(null);
        setFieldErrors({ [fe.field]: fe.message });
      } else {
        setErrorMessage(
          err instanceof ApiError &&
            !isServerFailure(err) &&
            err.code !== 'validationFailed' &&
            Object.prototype.hasOwnProperty.call(th, `error.${err.code}`)
            ? t(`error.${err.code}` as keyof typeof th)
            : fe.message,
        );
        setRetryAction(() => () => handleCreateBooking());
      }
    } finally {
      setWriting(false);
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
          {refusalMessage() ?? t('error.roleNotAllowed')}
        </div>
      </Show>

      <Show when={authorized() !== false && errorMessage()}>
        <div role="alert" class="xc-error-banner">
          <span>{errorMessage()}</span>
          <Show when={retryAction()}>
            <button
              type="button"
              data-action="retry"
              class="xc-retry-btn xc-btn-secondary"
              onClick={handleRetry}
            >
              {t('retry')}
            </button>
          </Show>
        </div>
      </Show>

      <Show when={authorized() === true}>
        <div class="xc-desk-bookings" data-screen="desk-bookings">
          <div class="xc-bookings-toolbar">
            <div class="xc-qr-search">
              <input
                name="qrToken"
                data-action="scan-qr"
                class="xc-qr-input"
                placeholder={t('scanQr')}
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
                {t('scanQr')}
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
                {t('newBooking')}
              </button>
            </div>
          </div>

          <div class="xc-slots-container">
            <Show
              when={allSlots().length > 0}
              fallback={<div class="xc-empty-state">{t('empty')}</div>}
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
                                {Object.prototype.hasOwnProperty.call(th, `status.${b.status}`)
                                  ? t(`status.${b.status}` as keyof typeof th)
                                  : t('statusUnknown')}
                              </span>
                            </div>

                            <div class="xc-booking-actions">
                              <Show when={b.status === 'CONFIRMED'}>
                                <button
                                  type="button"
                                  data-action="check-in"
                                  class="xc-checkin-btn xc-btn-primary"
                                  disabled={writing()}
                                  onClick={() => handleCheckIn(b.id)}
                                >
                                  {t('checkIn')}
                                </button>
                              </Show>
                              <Show when={b.status === 'CONFIRMED' || b.status === 'PENDING_CUSTOMER_CONFIRMATION'}>
                                <button
                                  type="button"
                                  data-action="cancel"
                                  class="xc-cancel-btn xc-btn-secondary"
                                  onClick={() => openCancel(b.id)}
                                >
                                  {t('cancel')}
                                </button>
                                <button
                                  type="button"
                                  data-action="reschedule"
                                  class="xc-reschedule-btn xc-btn-secondary"
                                  onClick={() => openReschedule(b.id)}
                                >
                                  {t('reschedule')}
                                </button>
                              </Show>
                              <Show when={b.status === 'CONFIRMED' || b.status === 'CHECKED_IN' || b.status === 'PENDING_CUSTOMER_CONFIRMATION'}>
                                <button
                                  type="button"
                                  name="reissueQr"
                                  data-action="reissue-qr"
                                  class="xc-btn-secondary"
                                  disabled={writing()}
                                  onClick={() => handleReissueQr(b.id, b.bookingNo)}
                                >
                                  {t('reissueQr')}
                                </button>
                              </Show>
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
                <h3 class="xc-modal-title">{t('cancel')}</h3>
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
                        {t('retry')}
                      </button>
                    </Show>
                  </div>
                </Show>
                <div class="xc-form-group">
                  <label for="cancel-reason-input">{t('reasonRequired')}</label>
                  <input
                    id="cancel-reason-input"
                    name="reason"
                    class="xc-cancel-reason"
                    maxLength={200}
                    value={cancelReason()}
                    onInput={(e) => {
                      setCancelReason(e.currentTarget.value);
                      setCancelReasonError(null);
                    }}
                    placeholder={t('reasonRequired')}
                  />
                  <div>{cancelReason().length} / 200</div>
                  <Show when={cancelReasonError()}>
                    {(msg) => (
                      <div role="alert" class="field-error">
                        {msg()}
                      </div>
                    )}
                  </Show>
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
                    disabled={writing()}
                    onClick={() => handleConfirmCancel()}
                  >
                    {t('cancel')}
                  </button>
                </div>
              </div>
            </div>
          </Show>

          {/* Reschedule Dialog */}
          <Show when={reschedulingBookingId()}>
            <div class="xc-modal-overlay">
              <div class="xc-modal">
                <h3 class="xc-modal-title">{t('reschedule')}</h3>
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
                        {t('retry')}
                      </button>
                    </Show>
                  </div>
                </Show>
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
                    disabled={writing()}
                    onClick={() => handleConfirmReschedule()}
                  >
                    {t('reschedule')}
                  </button>
                </div>
              </div>
            </div>
          </Show>

          {/* New Booking Modal */}
          <Show when={showNewBookingModal()}>
            <div class="xc-modal-overlay">
              <div class="xc-modal">
                <h3 class="xc-modal-title">{t('newBooking')}</h3>
                <Show when={errorMessage()}>
                  <div role="alert" class="xc-error-banner">
                    <span>{errorMessage()}</span>
                    <Show when={retryAction()}>
                      <button
                        type="button"
                        data-action="retry"
                        class="xc-retry-btn xc-btn-secondary"
                        disabled={writing()}
                        onClick={handleRetry}
                      >
                        {t('retry')}
                      </button>
                    </Show>
                  </div>
                </Show>
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
                      onInput={(e) => {
                        setFormSlotId(e.currentTarget.value);
                        clearFieldError('slotId');
                      }}
                      onChange={(e) => {
                        setFormSlotId(e.currentTarget.value);
                        clearFieldError('slotId');
                      }}
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
                    <Show when={fieldErrors().slotId}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
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
                    <select
                      id="new-booking-currency"
                      name="currency"
                      value={formCurrency()}
                      onInput={(e) => {
                        setFormCurrency(e.currentTarget.value);
                        clearFieldError('currency');
                      }}
                      onChange={(e) => {
                        setFormCurrency(e.currentTarget.value);
                        clearFieldError('currency');
                      }}
                    >
                      <For each={currencies()}>
                        {(code) => <option value={code}>{code}</option>}
                      </For>
                    </select>
                    <Show when={fieldErrors().currency}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group" data-field="quantity">
                    <label for="new-booking-quantity">Quantity</label>
                    <input
                      id="new-booking-quantity"
                      type="number"
                      name="quantity"
                      value={formQuantity()}
                      aria-invalid={quantityInvalid() || Boolean(fieldErrors().quantity) ? 'true' : undefined}
                      onInput={(e) => {
                        setFormQuantity(e.currentTarget.value);
                        setQuantityInvalid(false);
                        clearFieldError('quantity');
                      }}
                      onChange={(e) => {
                        setFormQuantity(e.currentTarget.value);
                        setQuantityInvalid(false);
                        clearFieldError('quantity');
                      }}
                    />
                    <Show when={quantityInvalid() ? t('error.validationFailed') : fieldErrors().quantity}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-display-name">Customer Name</label>
                    <input
                      id="new-booking-display-name"
                      name="displayName"
                      maxLength={100}
                      value={formDisplayName()}
                      onInput={(e) => {
                        setFormDisplayName(e.currentTarget.value);
                        clearFieldError('displayName');
                      }}
                      onChange={(e) => {
                        setFormDisplayName(e.currentTarget.value);
                        clearFieldError('displayName');
                      }}
                    />
                    <Show when={fieldErrors().displayName}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-phone">Customer Phone</label>
                    <input
                      id="new-booking-phone"
                      type="tel"
                      name="phone"
                      value={formPhone()}
                      onInput={(e) => {
                        setFormPhone(e.currentTarget.value);
                        clearFieldError('phone');
                      }}
                      onChange={(e) => {
                        setFormPhone(e.currentTarget.value);
                        clearFieldError('phone');
                      }}
                    />
                    <Show when={fieldErrors().phone}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-referral">Referral Code</label>
                    <input
                      id="new-booking-referral"
                      name="referralCode"
                      class="xc-referral-input"
                      value={formReferralCode()}
                      onInput={(e) => {
                        setFormReferralCode(e.currentTarget.value);
                        clearFieldError('referralCode');
                      }}
                      onChange={(e) => {
                        setFormReferralCode(e.currentTarget.value);
                        clearFieldError('referralCode');
                      }}
                    />
                    <Show when={fieldErrors().referralCode}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-modal-actions">
                    <button
                      type="button"
                      class="xc-btn-secondary"
                      disabled={writing()}
                      onClick={() => setShowNewBookingModal(false)}
                    >
                      Close
                    </button>
                    <button
                      type="submit"
                      data-action="submit-booking"
                      class="xc-submit-booking xc-btn-primary"
                      disabled={writing()}
                      onClick={handleSubmitNewBooking}
                    >
                      {t('newBooking')}
                    </button>
                  </div>
                </form>
              </div>
            </div>
          </Show>

          {/* QR Reissue Dialog */}
          <Show when={qrDialogData()}>
            {(data) => (
              <div class="xc-modal-overlay">
                <div class="xc-modal">
                  <h3 class="xc-modal-title">{t('reissueQr')}</h3>
                  <div class="xc-form-group">
                    <span class="xc-booking-no">{data().bookingNo}</span>
                  </div>
                  <Show
                    when={data().qrToken}
                    fallback={
                      <div class="xc-form-group">
                        <p>{t('alreadyIssued')}</p>
                      </div>
                    }
                  >
                    {(token) => (
                      <div class="xc-form-group">
                        <QrCode value={token()} label={data().bookingNo} />
                      </div>
                    )}
                  </Show>
                  <div class="xc-modal-actions">
                    <button
                      type="button"
                      class="xc-btn-secondary"
                      onClick={() => setQrDialogData(null)}
                    >
                      Close
                    </button>
                  </div>
                </div>
              </div>
            )}
          </Show>
        </div>
      </Show>
    </>
  );
}
