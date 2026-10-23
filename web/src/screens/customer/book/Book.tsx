// APP-16 customer book screen (design customer-app.md §4.5, §9.1 C02, #985).
import {
  createEffect,
  createSignal,
  onMount,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { fill, screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { readHostOrg } from '../../../shared/api/public-host';
import { checkDecimal, numeric } from '../../../shared/ui/bounded';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { QrCode } from '../../../shared/ui/QrCode';
import type { BookingSlotItemDto, BookingSlotsData } from '../../../shared/api/generated/booking_slots';
import type { BookingCreateInput, BookingCreateOutput } from '../../../shared/api/generated/bookings';
import { th, en, zh } from './i18n';
import './book.css';

export const BOOKING_SLOTS_VIEW_ROUTE = '/api/v3/views/booking_slots';
export const BOOKING_CREATE_COMMAND_ROUTE = '/api/bookings';

export interface BookScreenProps {
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
  [key: string]: unknown;
}

export interface PublicBranch {
  id: string;
  code: string;
  name: string;
  timezone: string;
  bookingEnabled?: boolean;
  [key: string]: unknown;
}

export type BookingSlot = BookingSlotItemDto;

export interface PlannerPreviewResult {
  thb: string;
  rate?: string;
  unitSize?: string;
  stale?: boolean;
  sourceDate?: string;
  [key: string]: unknown;
}

export type BookingCreateBody = Omit<Partial<BookingCreateInput>, 'quantity'> & {
  quantity?: string | number;
} & Record<string, unknown>;

const QUANTITY_BOUND = numeric(20, 4, 'positive');

export type BookingCreateResult = Partial<BookingCreateOutput> & {
  id: string;
  status: string;
  [key: string]: unknown;
};

export function getTodayInTimezone(timeZone: string): string {
  return new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(new Date());
}

export function formatSlotTime(start: string, end: string, timeZone: string): string {
  const fmt = new Intl.DateTimeFormat('en-GB', {
    timeZone,
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
  const startTime = fmt.format(new Date(start));
  const endTime = end ? fmt.format(new Date(end)) : '';
  return endTime ? `${startTime} – ${endTime}` : startTime;
}

export function BookScreen(props: BookScreenProps): JSX.Element {
  const t = screenT('customer.book', { th, en, zh });
  const fetchFn = props.fetch ?? ((url, init) => window.fetch(url, init));
  const api = new ApiClient(async (url, init) => {
    try {
      const res = await fetchFn(url, init);
      if (url === BOOKING_CREATE_COMMAND_ROUTE && res.status >= 500) {
        setSubmitting(false);
      }
      return res;
    } catch (err) {
      if (url === BOOKING_CREATE_COMMAND_ROUTE) {
        setSubmitting(false);
      }
      throw err;
    }
  });

  const bookError = (err: unknown): string => {
    if (err instanceof ApiError && !isServerFailure(err)) {
      const key = `error.${err.code}` as keyof typeof th;
      if (Object.prototype.hasOwnProperty.call(th, key)) {
        return t(key);
      }
    }
    return wordedError(err);
  };

  const searchParams = typeof window !== 'undefined' ? new URLSearchParams(window.location.search) : new URLSearchParams();
  const initialRef = searchParams.get('ref') ?? '';
  const initialBranch = searchParams.get('branch') ?? (typeof props.branchId === 'string' ? props.branchId : '');

  const [branchId, setBranchId] = createSignal(initialBranch);
  const [slotId, setSlotId] = createSignal('');
  const [serviceDate, setServiceDate] = createSignal('');
  const [minDate, setMinDate] = createSignal('');
  const [currency, setCurrency] = createSignal('USD');
  const [shopSide, setShopSide] = createSignal<'BUY' | 'SELL'>('BUY');
  const [quantity, setQuantity] = createSignal('');
  const [referralCode, setReferralCode] = createSignal(initialRef);

  const [branches, setBranches] = createSignal<PublicBranch[]>([]);
  const [branchesLoading, setBranchesLoading] = createSignal(true);
  const [branchesError, setBranchesError] = createSignal<string | null>(null);

  const [slots, setSlots] = createSignal<BookingSlot[]>([]);
  const [slotsLoading, setSlotsLoading] = createSignal(false);
  const [slotsError, setSlotsError] = createSignal<string | null>(null);

  const [preview, setPreview] = createSignal<PlannerPreviewResult | null>(null);

  const [submitting, setSubmitting] = createSignal(false);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});
  const [globalError, setGlobalError] = createSignal<string | null>(null);
  const [successData, setSuccessData] = createSignal<BookingCreateResult | null>(null);

  const clearFieldError = (field: string) => {
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
  };

  const fieldLabels = () => ({
    quantity: t('quantity'),
    slotId: t('slot'),
    branchId: t('branch'),
    currency: t('currency'),
    referralCode: t('referralCode'),
  });

  const createBooking = keyedWrite<BookingCreateBody, BookingCreateResult>(
    api,
    'POST',
    BOOKING_CREATE_COMMAND_ROUTE,
  );

  const loadBranches = async () => {
    setBranchesLoading(true);
    setBranchesError(null);
    try {
      const org =
        typeof props.org === 'string' && props.org
          ? props.org
          : await readHostOrg(api);
      const branchJson = await api.publicGet<{ branches?: PublicBranch[] }>(
        `/api/public/orgs/${encodeURIComponent(org)}/branches`,
      );
      const list = (branchJson?.branches ?? []).filter(
        (b: PublicBranch) => b.bookingEnabled !== false,
      );
      for (const b of list) {
        if (!b.timezone) {
          throw new Error('missing branch timezone');
        }
        getTodayInTimezone(b.timezone);
      }
      setBranches(list);
      setBranchesLoading(false);

      const qBranch = new URLSearchParams(window.location.search).get('branch') ?? (typeof props.branchId === 'string' ? props.branchId : '');
      const matchedBranch = qBranch ? (list.find((b) => b.id === qBranch) ?? list.find((b) => b.code === qBranch)) : undefined;
      const activeBranch = matchedBranch ? matchedBranch.id : (list.some((b) => b.id === branchId()) ? branchId() : '');
      if (activeBranch) {
        setBranchId(activeBranch);
        const br = list.find((b) => b.id === activeBranch);
        if (!br?.timezone) {
          throw new Error('missing branch timezone');
        }
        const today = getTodayInTimezone(br.timezone);
        setMinDate(today);
        const qDate = new URLSearchParams(window.location.search).get('date') ?? (typeof props.date === 'string' ? props.date : '');
        const dateToUse = qDate && qDate >= today ? qDate : today;
        setServiceDate(dateToUse);
        void loadSlots(activeBranch, dateToUse);
      } else {
        slotsSeq++;
        setBranchId('');
        setServiceDate('');
        setMinDate('');
        setSlots([]);
        setSlotId('');
        setSlotsLoading(false);
      }
    } catch {
      setBranches([]);
      setBranchesLoading(false);
      setBranchesError(t('error.loadFailed'));
    }
  };

  let slotsSeq = 0;
  let slotSelectEl: HTMLSelectElement | undefined;
  let branchSelectEl: HTMLSelectElement | undefined;

  createEffect(() => {
    branches();
    const id = branchId();
    if (branchSelectEl) {
      branchSelectEl.value = id;
    }
  });

  createEffect(() => {
    slots();
    const id = slotId();
    if (slotSelectEl) {
      slotSelectEl.value = id;
    }
  });

  const loadSlots = async (overrideBranch?: string, overrideDate?: string) => {
    const seq = ++slotsSeq;
    const b = overrideBranch !== undefined ? overrideBranch : branchId();
    const d = overrideDate !== undefined ? overrideDate : serviceDate();
    if (!b || !d) {
      setSlots([]);
      setSlotId('');
      setSlotsLoading(false);
      return;
    }
    setSlotsLoading(true);
    setSlotsError(null);
    try {
      const res = await api.view<BookingSlotsData>('booking_slots', {
        branch: b,
        filter: { date: d },
      });
      if (seq !== slotsSeq) return;
      const list: BookingSlot[] = res.data?.slots ?? [];
      setSlots(list);
      if (!list.some((s) => s.id === slotId())) {
        setSlotId('');
      }
      setSlotsLoading(false);
    } catch {
      if (seq !== slotsSeq) return;
      setSlots([]);
      setSlotId('');
      setSlotsLoading(false);
      setSlotsError(t('error.loadFailed'));
    }
  };

  onMount(() => {
    void loadBranches();
  });

  let previewSeq = 0;
  let createFailed = false;

  const updatePreview = async () => {
    const seq = ++previewSeq;
    const b = branchId();
    const c = currency();
    const q = quantity().trim();
    const s = shopSide();

    if (!q) {
      setPreview(null);
      return;
    }

    const checked = checkDecimal(q, QUANTITY_BOUND);
    if (!checked.ok) {
      setPreview(null);
      setFieldErrors((prev) => ({
        ...prev,
        quantity: fieldError(
          new ApiError({ status: 400, code: 'validationFailed', field: 'quantity', message: '' }),
          fieldLabels(),
        ).message,
      }));
      return;
    }

    if (!b) {
      setPreview(null);
      return;
    }

    try {
      const data = await api.act<
        {
          branch: string;
          currency: string;
          customerSide: 'CUSTOMER_BUY' | 'CUSTOMER_SELL';
          amount: { quantity: string };
        },
        PlannerPreviewResult
      >('compute', 'planner_preview', {
        branch: b,
        currency: c,
        customerSide: s === 'BUY' ? 'CUSTOMER_BUY' : 'CUSTOMER_SELL',
        amount: { quantity: checked.text },
      });
      if (seq !== previewSeq) return;
      setPreview(data);
      if (!createFailed) {
        setGlobalError(null);
      }
    } catch (err) {
      if (seq !== previewSeq) return;
      setPreview(null);
      if (!submitting() && !createFailed) {
        setGlobalError(bookError(err));
      }
    }
  };

  let submitSeq = 0;

  const handleSubmit = async (e?: Event) => {
    if (e) e.preventDefault();
    if (submitting()) return;
    createFailed = false;
    setGlobalError(null);
    setFieldErrors({});
    setSuccessData(null);

    // Read current form values from signals or DOM
    const currentBranch =
      branchId() || (document.querySelector('select[name="branchId"]') as HTMLSelectElement)?.value || '';
    const currentSlot =
      slotId() || (document.querySelector('select[name="slotId"]') as HTMLSelectElement)?.value || '';
    const currentSide =
      shopSide() ||
      ((document.querySelector('select[name="shopSide"]') as HTMLSelectElement)?.value as 'BUY' | 'SELL') ||
      'BUY';
    const currentCurrency =
      currency() || (document.querySelector('select[name="currency"]') as HTMLSelectElement)?.value || 'USD';
    const currentQtyStr = (
      quantity() ||
      (document.querySelector('input[name="quantity"]') as HTMLInputElement)?.value ||
      ''
    ).trim();
    const currentRef =
      referralCode() !== undefined
        ? referralCode()
        : (document.querySelector('input[name="referralCode"]') as HTMLInputElement)?.value ?? '';

    const checkedQty = checkDecimal(currentQtyStr, QUANTITY_BOUND);
    if (!checkedQty.ok) {
      setFieldErrors({
        quantity: fieldError(
          new ApiError({ status: 400, code: 'validationFailed', field: 'quantity', message: '' }),
          fieldLabels(),
        ).message,
      });
      return;
    }

    const seq = ++submitSeq;
    setSubmitting(true);

    const payload: BookingCreateBody = {
      branch: currentBranch,
      branchId: currentBranch,
      slotId: currentSlot,
      shopSide: currentSide,
      currency: currentCurrency,
      quantity: checkedQty.text,
    };

    if (currentRef !== '') {
      payload.referralCode = currentRef;
    }

    try {
      const data = await createBooking.send(payload);
      if (seq !== submitSeq) return;
      setSuccessData(data);
    } catch (err) {
      if (seq !== submitSeq) return;
      if (err instanceof ApiError && !isServerFailure(err)) {
        if (
          err.code === 'invalidReferralCode' ||
          (err.code === 'validationFailed' && err.field === 'referralCode')
        ) {
          setFieldErrors({ referralCode: t('error.invalidReferralCode') });
          return;
        }
        if (err.code === 'slotUnavailable' || err.code === 'capacityFull') {
          void loadSlots(currentBranch, serviceDate());
        }
        const fe = fieldError(err, fieldLabels());
        if (fe.field) {
          setFieldErrors({
            [fe.field]: fe.field === 'referralCode' ? t('error.invalidReferralCode') : fe.message,
          });
          return;
        }
      }
      createFailed = true;
      setGlobalError(bookError(err));
    } finally {
      if (seq === submitSeq) {
        setSubmitting(false);
      }
    }
  };

  return (
    <div class="xc-book-screen" data-testid="book-screen">
      <header class="xc-book-header">
        <h1 class="xc-book-title">{t('title')}</h1>
        <p class="xc-book-subtitle">{t('subtitle')}</p>
      </header>

      <Show when={globalError()}>
        <div class="alert alert-error" role="alert">
          {globalError()}
        </div>
      </Show>

      <Show when={successData()}>
        <div class="xc-book-success alert-success" role="status">
          <p>{t('bookingSuccess')}</p>
          <p>{fill(t('bookingNo'), { no: successData()!.bookingNo || successData()!.id })}</p>
          <Show when={successData()!.qrToken}>
            <div class="xc-booking-qr" data-qr-token={successData()!.qrToken!}>
              <QrCode
                value={successData()!.qrToken!}
                label={t('qrCode')}
                downloadName={`booking-${successData()!.bookingNo || successData()!.id}`}
                downloadLabel={t('downloadQr')}
              />
            </div>
            <p class="xc-book-qr-hint">{t('qrShowAtDesk')}</p>
          </Show>
          <a class="xc-book-bookings-link" href="/app/bookings">
            {t('viewMyBookings')}
          </a>
        </div>
      </Show>

      <form class="xc-book-form" onSubmit={handleSubmit}>
        <div class="xc-book-field field" data-field="branchId">
          <label class="xc-book-label" for="branchId">
            {t('branch')}
          </label>

          <Show when={branchesLoading()}>
            <div class="xc-book-loading xc-book-branches-loading" role="status">
              {t('loading')}
            </div>
          </Show>

          <Show when={branchesError()}>
            <div class="xc-book-field-error alert alert-error xc-book-branches-error" role="alert">
              <span>{branchesError()}</span>
              <button type="button" class="xc-book-retry-btn" onClick={loadBranches}>
                {t('retry')}
              </button>
            </div>
          </Show>

          <Show when={!branchesLoading() && !branchesError() && branches().length === 0}>
            <div class="xc-book-empty xc-book-branches-empty" role="status">
              {t('noBranches')}
            </div>
          </Show>

          <select
            ref={branchSelectEl}
            id="branchId"
            name="branchId"
            required
            class="xc-book-select"
            data-testid="branch-select"
            value={branchId()}
            disabled={branchesLoading() || branches().length === 0}
            onChange={(e) => {
              const val = e.currentTarget.value;
              setBranchId(val);
              setSlotId('');
              clearFieldError('branchId');
              clearFieldError('slotId');
              void updatePreview();
              if (val) {
                const br = branches().find((b) => b.id === val);
                try {
                  if (!br?.timezone) {
                    throw new Error('missing branch timezone');
                  }
                  const today = getTodayInTimezone(br.timezone);
                  setMinDate(today);
                  const qDate = new URLSearchParams(window.location.search).get('date') ?? (typeof props.date === 'string' ? props.date : '');
                  const chosenDate = qDate && qDate >= today ? qDate : today;
                  setServiceDate(chosenDate);
                  void loadSlots(val, chosenDate);
                } catch {
                  setBranchesError(t('error.loadFailed'));
                }
              } else {
                slotsSeq++;
                setServiceDate('');
                setMinDate('');
                setSlots([]);
                setSlotsLoading(false);
              }
            }}
          >
            <option value="">{branches().length === 0 ? t('noBranches') : t('selectBranch')}</option>
            <For each={branches()}>
              {(b) => (
                <option value={b.id}>
                  {b.name} ({b.code})
                </option>
              )}
            </For>
          </select>
          <Show when={fieldErrors().branchId}>
            <div class="field-error xc-book-error" data-field-error="branchId" role="alert">
              {fieldErrors().branchId}
            </div>
          </Show>
        </div>

        <div class="xc-book-field field" data-field="serviceDate">
          <label class="xc-book-label" for="serviceDate">
            {t('date')}
          </label>
          <input
            id="serviceDate"
            name="serviceDate"
            type="date"
            class="xc-book-input"
            data-testid="date-input"
            value={serviceDate()}
            min={minDate() || undefined}
            disabled={!branchId()}
            onInput={(e) => {
              const newDate = e.currentTarget.value;
              if (newDate === serviceDate()) return;
              setSlotId('');
              setServiceDate(newDate);
              if (branchId() && newDate) {
                void loadSlots(branchId(), newDate);
              } else {
                slotsSeq++;
                setSlots([]);
                setSlotsLoading(false);
              }
            }}
            onChange={(e) => {
              const newDate = e.currentTarget.value;
              if (newDate === serviceDate()) return;
              setSlotId('');
              setServiceDate(newDate);
              if (branchId() && newDate) {
                void loadSlots(branchId(), newDate);
              } else {
                slotsSeq++;
                setSlots([]);
                setSlotsLoading(false);
              }
            }}
          />
        </div>

        <div class="xc-book-field field" data-field="slotId">
          <label class="xc-book-label" for="slotId">
            {t('slot')}
          </label>

          <Show when={slotsLoading()}>
            <div class="xc-book-loading xc-book-slots-loading" role="status">
              {t('loading')}
            </div>
          </Show>

          <Show when={slotsError()}>
            <div class="xc-book-field-error alert alert-error xc-book-slots-error" role="alert">
              <span>{slotsError()}</span>
              <button type="button" class="xc-book-retry-btn" onClick={() => loadSlots(branchId(), serviceDate())}>
                {t('retry')}
              </button>
            </div>
          </Show>

          <Show when={!slotsLoading() && !slotsError() && slots().length === 0}>
            <div class="xc-book-empty xc-book-slots-empty" role="status">
              {t('noSlots')}
            </div>
          </Show>

          <select
            ref={slotSelectEl}
            id="slotId"
            name="slotId"
            required
            class="xc-book-select"
            data-testid="slot-select"
            value={slotId()}
            disabled={slotsLoading() || slots().length === 0}
            onChange={(e) => {
              setSlotId(e.currentTarget.value);
              clearFieldError('slotId');
            }}
          >
            <option value="">{slotsLoading() ? t('loading') : (slots().length === 0 ? t('noSlots') : t('selectSlot'))}</option>
            <For each={branches().find((b) => b.id === branchId()) ? slots() : []}>
              {(slot) => {
                const br = branches().find((b) => b.id === branchId())!;
                return (
                  <option
                    data-testid="slot-option"
                    value={slot.id}
                    disabled={slot.available === 0}
                  >
                    {formatSlotTime(slot.start, slot.end, br.timezone)} ({fill(t('availableCount'), { count: slot.available })})
                  </option>
                );
              }}
            </For>
          </select>
          <Show when={fieldErrors().slotId}>
            <div class="field-error xc-book-error" data-field-error="slotId" role="alert">
              {fieldErrors().slotId}
            </div>
          </Show>
        </div>

        <div class="xc-book-field field">
          <label class="xc-book-label" for="side">
            {t('side')}
          </label>
          <select
            id="side"
            name="shopSide"
            class="xc-book-select"
            value={shopSide()}
            onChange={(e) => {
              setShopSide(e.currentTarget.value as 'BUY' | 'SELL');
              void updatePreview();
            }}
          >
            <option value="BUY">{t('buy')}</option>
            <option value="SELL">{t('sell')}</option>
          </select>
        </div>

        <div class="xc-book-field field" data-field="currency">
          <label class="xc-book-label" for="currency">
            {t('currency')}
          </label>
          <select
            id="currency"
            name="currency"
            class="xc-book-select"
            data-testid="currency-select"
            value={currency()}
            onChange={(e) => {
              setCurrency(e.currentTarget.value);
              clearFieldError('currency');
              void updatePreview();
            }}
          >
            <option value="USD">USD</option>
            <option value="EUR">EUR</option>
            <option value="JPY">JPY</option>
            <option value="GBP">GBP</option>
            <option value="SGD">SGD</option>
          </select>
          <Show when={fieldErrors().currency}>
            <div class="field-error xc-book-error" data-field-error="currency" role="alert">
              {fieldErrors().currency}
            </div>
          </Show>
        </div>

        <div class="xc-book-field field" data-field="quantity">
          <label class="xc-book-label" for="quantity">
            {t('quantity')}
          </label>
          <input
            id="quantity"
            name="quantity"
            class="xc-book-input"
            data-testid="quantity-input"
            type="number"
            min="1"
            step="any"
            placeholder={t('quantityPlaceholder')}
            value={quantity()}
            onInput={(e) => {
              setQuantity(e.currentTarget.value);
              clearFieldError('quantity');
              void updatePreview();
            }}
            aria-invalid={fieldErrors().quantity ? 'true' : undefined}
          />
          <Show when={fieldErrors().quantity}>
            <div class="field-error xc-book-error" data-field-error="quantity" role="alert">
              {fieldErrors().quantity}
            </div>
          </Show>
        </div>

        <div class="xc-book-field field" data-field="referralCode">
          <label class="xc-book-label" for="referralCode">
            {t('referralCode')}
          </label>
          <input
            id="referralCode"
            name="referralCode"
            class="xc-book-input"
            data-testid="referral-code"
            type="text"
            placeholder={t('referralCodePlaceholder')}
            value={referralCode()}
            onInput={(e) => {
              setReferralCode(e.currentTarget.value);
              clearFieldError('referralCode');
            }}
            aria-invalid={fieldErrors().referralCode ? 'true' : undefined}
          />
          <Show when={fieldErrors().referralCode}>
            <div class="field-error xc-book-error" data-field-error="referralCode" role="alert">
              {fieldErrors().referralCode}
            </div>
          </Show>
        </div>

        <Show when={preview()}>
          <div class="planner-preview xc-book-preview" data-testid="planner-preview">
            <div class="preview-amount xc-book-preview-amount">
              <span class="preview-label xc-book-preview-label">{t('estimatedThb')}: </span>
              <MoneyText value={preview()!.thb} currency="THB" />
            </div>
            <p class="preview-notice xc-book-preview-notice">{t('estimateNotice')}</p>
          </div>
        </Show>

        <button
          type="submit"
          class="xc-book-submit"
          data-action="book"
          data-testid="book-submit"
          disabled={submitting()}
        >
          {submitting() ? t('submitting') : t('submit')}
        </button>
      </form>
    </div>
  );
}

export default BookScreen;
