import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { BookingSlotItemDto, BookingSlotsData } from '../../../shared/api/generated/booking_slots';
import type { DeskBookingItemDto, DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { DeskConfig, DeskRate } from '../../../shared/api/generated/desk_config';
import { publishedCodes } from '../published-rates';
import type {
  BookingCancelOutput,
  BookingCheckInOutput,
  BookingCreateOutput,
  BookingQrReissueOutput,
  BookingRescheduleOutput,
} from '../../../shared/api/generated/bookings';
import type { MeView } from '../../../shared/api/generated/me';
import type { QuoteDto } from '../../../shared/api/generated/quote';
import type { QuotesCommandInput } from '../../../shared/api/generated/quotes';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { parseRequestQr } from '../../../shared/queue/request-qr';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { QrCode } from '../../../shared/ui/QrCode';
import { BenefitLines } from '../benefits/BenefitLines';
import { newKey } from '../save';
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
const VIEW_BOOKING_SLOTS = 'booking_slots';
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

function formatSlotDate(startAt?: string, timeZone = 'Asia/Bangkok'): string {
  if (!startAt) return '';
  try {
    return new Intl.DateTimeFormat('th-TH', {
      timeZone,
      day: 'numeric',
      month: 'short',
    }).format(new Date(startAt));
  } catch {
    return '';
  }
}

function formatBangkokDate(iso?: string): string {
  const d = iso ? new Date(iso) : new Date();
  const valid = Number.isNaN(d.getTime()) ? new Date() : d;
  return new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Bangkok' }).format(valid);
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
  const [rescheduleDate, setRescheduleDate] = createSignal('');
  const [targetSlotId, setTargetSlotId] = createSignal('');
  const [rescheduleSlots, setRescheduleSlots] = createSignal<BookingSlotItemDto[]>([]);
  const [rescheduleSlotsError, setRescheduleSlotsError] = createSignal<string | null>(null);
  let rescheduleSlotsSeq = 0;

  // QR reissue dialog state
  const [qrDialogData, setQrDialogData] = createSignal<{ bookingNo: string; qrToken: string | null } | null>(null);

  // New booking modal state
  const [showNewBookingModal, setShowNewBookingModal] = createSignal(false);
  const [rates, setRates] = createSignal<DeskRate[] | null>(null);
  const activeSide = () => (formShopSide() === 'BUY' ? 'BUY' : 'SELL');
  const currencies = () => {
    const r = rates();
    if (r === null) return ['USD'];
    return publishedCodes(r, activeSide());
  };
  const [formBranchId, setFormBranchId] = createSignal('');
  const [formDate, setFormDate] = createSignal('');
  const [formSlotId, setFormSlotId] = createSignal('');
  const [newBookingSlots, setNewBookingSlots] = createSignal<BookingSlotItemDto[]>([]);
  const [newBookingSlotsError, setNewBookingSlotsError] = createSignal<string | null>(null);
  let newBookingSlotsSeq = 0;
  const [formShopSide, setFormShopSide] = createSignal('SELL');
  const [formCurrency, setFormCurrency] = createSignal('USD');

  function handleSideChange(val: string): void {
    if (val === formShopSide()) return;
    setFormShopSide(val);
    const list = currencies();
    if (!list.includes(formCurrency())) {
      setFormCurrency(list[0] ?? '');
    }
    void requestBookingQuote();
  }
  const [formQuantity, setFormQuantity] = createSignal('');
  const [quantityInvalid, setQuantityInvalid] = createSignal(false);
  const [formDisplayName, setFormDisplayName] = createSignal('');
  const [formPhone, setFormPhone] = createSignal('');
  const [formReferralCode, setFormReferralCode] = createSignal('');
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});
  const [bookingQuote, setBookingQuote] = createSignal<QuoteDto | undefined>(undefined);
  let bookingQuoteSeq = 0;

  async function requestBookingQuote(rethrow = false): Promise<QuoteDto | undefined> {
    const slot = formSlotId().trim();
    const shopSide = formShopSide().trim();
    const currency = formCurrency().trim();
    const rawQty = formQuantity().trim();
    const qty = Number(rawQty);
    if (!slot || !shopSide || !currency || !/^\d+$/.test(rawQty) || !Number.isSafeInteger(qty) || qty <= 0) {
      bookingQuoteSeq += 1;
      setBookingQuote(undefined);
      return undefined;
    }
    const mine = ++bookingQuoteSeq;
    setBookingQuote(undefined);
    const targetBranch = formBranchId() || branchId();
    const body: QuotesCommandInput = {
      branch: targetBranch,
      branchId: targetBranch,
      customerId: null,
      channel: 'BOOKING',
      lines: [
        {
          currency,
          shopSide,
          quantity: String(qty),
          feeThb: null,
          denominationGroup: null,
        },
      ],
    };
    try {
      const answered = await api.command<QuotesCommandInput, QuoteDto>(
        'POST',
        '/api/quotes',
        body,
        newKey(),
      );
      if (mine !== bookingQuoteSeq) return bookingQuote();
      setBookingQuote(answered);
      return answered;
    } catch (err) {
      if (mine === bookingQuoteSeq) {
        setBookingQuote(undefined);
      }
      if (rethrow) throw err;
      return undefined;
    }
  }

  function clearFieldError(field: string) {
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
  }

  async function loadNewBookingSlots(branch: string, date: string) {
    const seq = ++newBookingSlotsSeq;
    setNewBookingSlotsError(null);
    try {
      const env = await api.view<BookingSlotsData>(VIEW_BOOKING_SLOTS, {
        branch: branch.trim() || undefined,
        filter: { date: date || null },
      });
      if (seq !== newBookingSlotsSeq) return;
      const open = (env.data?.slots ?? []).filter((s) => s.available > 0);
      setNewBookingSlots(open);
      const cur = formSlotId();
      const nextSlot = cur && open.some((s) => s.id === cur) ? cur : (open[0]?.id ?? '');
      if (nextSlot !== cur) {
        setFormSlotId(nextSlot);
        void requestBookingQuote();
      }
    } catch {
      if (seq !== newBookingSlotsSeq) return;
      setNewBookingSlots([]);
      setNewBookingSlotsError(t('dialog.slotsLoadFailed'));
      if (formSlotId() !== '') {
        setFormSlotId('');
        void requestBookingQuote();
      }
    }
  }

  async function loadRescheduleSlots(date: string, bookingId: string | null) {
    const seq = ++rescheduleSlotsSeq;
    setRescheduleSlotsError(null);
    try {
      const env = await api.view<BookingSlotsData>(VIEW_BOOKING_SLOTS, {
        branch: branchId().trim() || undefined,
        filter: { date: date || null },
      });
      if (seq !== rescheduleSlotsSeq) return;
      const ownSlotId = items().find((b) => b.id === bookingId)?.slotId;
      const open = (env.data?.slots ?? []).filter((s) => s.available > 0 && s.id !== ownSlotId);
      setRescheduleSlots(open);
      setTargetSlotId(open[0]?.id ?? '');
    } catch {
      if (seq !== rescheduleSlotsSeq) return;
      setRescheduleSlots([]);
      setRescheduleSlotsError(t('dialog.slotsLoadFailed'));
      setTargetSlotId('');
    }
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
          const cfgRates = cfgEnv.data?.rates ?? [];
          setRates(cfgRates);
          const list = currencies();
          if (!list.includes(formCurrency())) {
            setFormCurrency(list[0] ?? '');
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
  let activeCreateQuoteId = '';
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
    const date = formatBangkokDate(booking?.startAt);
    setRescheduleDate(date);
    setRescheduleSlots([]);
    setRescheduleSlotsError(null);
    setTargetSlotId('');
    void loadRescheduleSlots(date, bookingId);
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
    const today = formatBangkokDate();
    setFormDate(today);
    setNewBookingSlots([]);
    setNewBookingSlotsError(null);
    setFormSlotId('');
    setShowNewBookingModal(true);
    void loadNewBookingSlots(formBranchId() || branchId(), today);
  }

  function resetNewBookingForm() {
    bookingQuoteSeq += 1;
    setBookingQuote(undefined);
    setFormQuantity('');
    setQuantityInvalid(false);
    setFormDisplayName('');
    setFormPhone('');
    setFormReferralCode('');
    setFieldErrors({});
    const list = currencies();
    if (!list.includes(formCurrency())) {
      setFormCurrency(list[0] ?? '');
    }
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
      let quote = bookingQuote();
      if (!quote) {
        quote = await requestBookingQuote(true);
      }
      if (!quote) {
        readErrorActive = false;
        setErrorMessage(t('error.validationFailed'));
        return;
      }
      if (activeCreateQuoteId !== quote.id) {
        createWrite.reset();
        activeCreateQuoteId = quote.id;
      }
      payload.quoteId = quote.id;
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
        quoteId: t('field.quoteId'),
      });
      if (fe.field === 'quoteId') {
        bookingQuoteSeq += 1;
        setBookingQuote(undefined);
        createWrite.reset();
        activeCreateQuoteId = '';
        setErrorMessage(fe.message);
        setRetryAction(() => () => handleCreateBooking());
      } else if (fe.field) {
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
                              <Show when={b.status === 'CONFIRMED' || b.status === 'PENDING_CUSTOMER_CONFIRMATION' || b.status === 'CHECKED_IN' || b.status === 'IN_SERVICE'}>
                                <button
                                  type="button"
                                  data-action="cancel"
                                  class="xc-cancel-btn xc-btn-secondary"
                                  onClick={() => openCancel(b.id)}
                                >
                                  {t('cancel')}
                                </button>
                              </Show>
                              <Show when={b.status === 'CONFIRMED' || b.status === 'PENDING_CUSTOMER_CONFIRMATION'}>
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
                    {t('close')}
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
                  <label for="reschedule-date">{t('dialog.date')}</label>
                  <input
                    id="reschedule-date"
                    type="date"
                    value={rescheduleDate()}
                    onInput={(e) => {
                      if (e.currentTarget.value === rescheduleDate()) return;
                      setRescheduleDate(e.currentTarget.value);
                      void loadRescheduleSlots(e.currentTarget.value, reschedulingBookingId());
                    }}
                    onChange={(e) => {
                      if (e.currentTarget.value === rescheduleDate()) return;
                      setRescheduleDate(e.currentTarget.value);
                      void loadRescheduleSlots(e.currentTarget.value, reschedulingBookingId());
                    }}
                  />
                </div>
                <div class="xc-form-group">
                  <label for="reschedule-slot-select">{t('dialog.slot')}</label>
                  <select
                    id="reschedule-slot-select"
                    name="targetSlotId"
                    value={targetSlotId()}
                    onInput={(e) => setTargetSlotId(e.currentTarget.value)}
                    onChange={(e) => setTargetSlotId(e.currentTarget.value)}
                  >
                    <Show
                      when={rescheduleSlots().length > 0}
                      fallback={
                        <option value="" disabled selected>
                          {t('dialog.noSlots')}
                        </option>
                      }
                    >
                      <For each={rescheduleSlots()}>
                        {(s) => (
                          <option value={s.id} selected={targetSlotId() === s.id}>
                            {s.start
                              ? `${formatSlotDate(s.start)} ${formatSlotTime(s.start, s.end)}`
                              : s.id}
                          </option>
                        )}
                      </For>
                    </Show>
                  </select>
                  <Show when={rescheduleSlotsError()}>
                    {(msg) => (
                      <div role="alert" class="field-error" data-row="slots-error">
                        {msg()}
                      </div>
                    )}
                  </Show>
                </div>
                <div class="xc-modal-actions">
                  <button
                    type="button"
                    class="xc-btn-secondary"
                    onClick={() => setReschedulingBookingId(null)}
                  >
                    {t('close')}
                  </button>
                  <button
                    type="button"
                    data-action="confirm-reschedule"
                    class="xc-confirm-reschedule xc-btn-primary"
                    disabled={writing() || !targetSlotId()}
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
                    <label for="new-booking-branch">{t('dialog.branch')}</label>
                    <input
                      id="new-booking-branch"
                      name="branchId"
                      value={formBranchId()}
                      onInput={(e) => {
                        if (e.currentTarget.value === formBranchId()) return;
                        setFormBranchId(e.currentTarget.value);
                        void loadNewBookingSlots(e.currentTarget.value || branchId(), formDate());
                        void requestBookingQuote();
                      }}
                      onChange={(e) => {
                        if (e.currentTarget.value === formBranchId()) return;
                        setFormBranchId(e.currentTarget.value);
                        void loadNewBookingSlots(e.currentTarget.value || branchId(), formDate());
                        void requestBookingQuote();
                      }}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-date">{t('dialog.date')}</label>
                    <input
                      id="new-booking-date"
                      type="date"
                      name="serviceDate"
                      value={formDate()}
                      onInput={(e) => {
                        if (e.currentTarget.value === formDate()) return;
                        setFormDate(e.currentTarget.value);
                        void loadNewBookingSlots(formBranchId() || branchId(), e.currentTarget.value);
                      }}
                      onChange={(e) => {
                        if (e.currentTarget.value === formDate()) return;
                        setFormDate(e.currentTarget.value);
                        void loadNewBookingSlots(formBranchId() || branchId(), e.currentTarget.value);
                      }}
                    />
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-slot">{t('dialog.slot')}</label>
                    <select
                      id="new-booking-slot"
                      name="slotId"
                      value={formSlotId()}
                      onInput={(e) => {
                        clearFieldError('slotId');
                        if (e.currentTarget.value === formSlotId()) return;
                        setFormSlotId(e.currentTarget.value);
                        void requestBookingQuote();
                      }}
                      onChange={(e) => {
                        clearFieldError('slotId');
                        if (e.currentTarget.value === formSlotId()) return;
                        setFormSlotId(e.currentTarget.value);
                        void requestBookingQuote();
                      }}
                    >
                      <Show
                        when={newBookingSlots().length > 0}
                        fallback={
                          <option value="" disabled selected>
                            {t('dialog.noSlots')}
                          </option>
                        }
                      >
                        <For each={newBookingSlots()}>
                          {(s) => (
                            <option value={s.id} selected={formSlotId() === s.id}>
                              {s.start
                                ? `${formatSlotDate(s.start)} ${formatSlotTime(s.start, s.end)}`
                                : s.id}
                            </option>
                          )}
                        </For>
                      </Show>
                    </select>
                    <Show when={newBookingSlotsError()}>
                      {(msg) => (
                        <div role="alert" class="field-error" data-row="slots-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                    <Show when={fieldErrors().slotId}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-side">{t('dialog.side')}</label>
                    <select
                      id="new-booking-side"
                      name="shopSide"
                      value={formShopSide()}
                      onInput={(e) => {
                        handleSideChange(e.currentTarget.value);
                      }}
                      onChange={(e) => {
                        handleSideChange(e.currentTarget.value);
                      }}
                    >
                      <option value="BUY">BUY</option>
                      <option value="SELL">SELL</option>
                    </select>
                  </div>

                  <div class="xc-form-group">
                    <label for="new-booking-currency">{t('dialog.currency')}</label>
                    <select
                      id="new-booking-currency"
                      name="currency"
                      value={formCurrency()}
                      onInput={(e) => {
                        clearFieldError('currency');
                        if (e.currentTarget.value === formCurrency()) return;
                        setFormCurrency(e.currentTarget.value);
                        void requestBookingQuote();
                      }}
                      onChange={(e) => {
                        clearFieldError('currency');
                        if (e.currentTarget.value === formCurrency()) return;
                        setFormCurrency(e.currentTarget.value);
                        void requestBookingQuote();
                      }}
                    >
                      <For each={currencies()}>
                        {(code) => <option value={code}>{code}</option>}
                      </For>
                    </select>
                    <Show when={currencies().length === 0}>
                      <div data-row="no-published-rate">
                        {t('noPublishedRate')}
                      </div>
                    </Show>
                    <Show when={fieldErrors().currency}>
                      {(msg) => (
                        <div role="alert" class="field-error">
                          {msg()}
                        </div>
                      )}
                    </Show>
                  </div>

                  <div class="xc-form-group" data-field="quantity">
                    <label for="new-booking-quantity">{t('dialog.quantity')}</label>
                    <input
                      id="new-booking-quantity"
                      type="number"
                      name="quantity"
                      value={formQuantity()}
                      aria-invalid={quantityInvalid() || Boolean(fieldErrors().quantity) ? 'true' : undefined}
                      onInput={(e) => {
                        setQuantityInvalid(false);
                        clearFieldError('quantity');
                        if (e.currentTarget.value === formQuantity()) return;
                        setFormQuantity(e.currentTarget.value);
                        void requestBookingQuote();
                      }}
                      onChange={(e) => {
                        setQuantityInvalid(false);
                        clearFieldError('quantity');
                        if (e.currentTarget.value === formQuantity()) return;
                        setFormQuantity(e.currentTarget.value);
                        void requestBookingQuote();
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
                    <label for="new-booking-display-name">{t('dialog.displayName')}</label>
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
                    <label for="new-booking-phone">{t('dialog.phone')}</label>
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
                    <label for="new-booking-referral">{t('dialog.referralCode')}</label>
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

                  <Show when={bookingQuote()}>
                    {(q) => (
                      <dl data-row="booking-estimate">
                        <For each={q().lines}>
                          {(line) => (
                            <div data-row="booking-estimate-thb">
                              <dt>{t('dialog.estimatedThb')}</dt>
                              <dd>
                                <MoneyText value={line.settleThb} currency="THB" />
                              </dd>
                            </div>
                          )}
                        </For>
                        <BenefitLines benefits={q().benefits} basis={q().benefitBasis} />
                      </dl>
                    )}
                  </Show>

                  <div class="xc-modal-actions">
                    <button
                      type="button"
                      class="xc-btn-secondary"
                      disabled={writing()}
                      onClick={() => {
                        resetNewBookingForm();
                        setShowNewBookingModal(false);
                      }}
                    >
                      {t('close')}
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
                      {t('close')}
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
