// V2-bookings (#463 §7; design §12.1, §12.6, §12.14, §12.17)
import {
  createEffect,
  createSignal,
  For,
  Show,
  untrack,
  type JSX,
} from 'solid-js';
import type { AdminMenuProps } from '../menus';
import { screenT, locale, errorText } from '../../../shared/i18n';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  AdminBookingRow,
  AdminBookingSettingsData,
} from '../../../shared/api/generated/admin_bookings';
import { openDialog, closeDialog } from '../parts/Dialog';
import { toast } from '../parts/toast';
import { refreshTick } from '../parts/refresh';
import { th, en } from './i18n';

function formatMoney(value: string | null | undefined): string {
  if (!value) return '';
  const isDecimal = /^-?\d+(\.\d+)?$/.test(value);
  if (!isDecimal) return value;
  const [intPart, decPart] = value.split('.');
  const isNegative = intPart.startsWith('-');
  const unsignedInt = isNegative ? intPart.slice(1) : intPart;
  const groupedInt = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${isNegative ? '-' : ''}${groupedInt}${decPart !== undefined ? `.${decPart}` : ''}`;
}

function getStatusText(status: string, t: (k: any) => string): string {
  switch (status) {
    case 'PENDING':
      return t('statusPending');
    case 'CONFIRMED':
      return t('statusConfirmed');
    case 'USED':
      return t('statusDone');
    case 'EXPIRED':
      return t('statusExpired');
    case 'CANCELLED':
      return t('statusCancelled');
    default:
      return status;
  }
}

function formatExpiry(expiresAt: string | null, t: (k: any) => string, loc: string): string {
  if (!expiresAt) {
    return t('pendingStaff');
  }
  return new Date(expiresAt).toLocaleString(loc);
}

export function BookingsScreen(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.bookings', { th, en });
  const api = new ApiClient();

  const [rows, setRows] = createSignal<AdminBookingRow[]>([]);
  const [settings, setSettings] = createSignal<AdminBookingSettingsData | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [searchTerm, setSearchTerm] = createSignal('');
  const [hasMore, setHasMore] = createSignal(false);

  let bookingsVersion: number | undefined;
  let settingsVersion: number | undefined;
  let currentBranch: string | undefined;
  let currentQ: string = '';

  async function fetchBookings(b: string, since?: number, q?: string) {
    if (rows().length === 0 && since === undefined) {
      setLoading(true);
    }
    setError(null);
    try {
      const filter = q ? { q } : undefined;
      const ans = await api.view<AdminBookingRow[], { q?: string }>('admin_bookings', {
        branch: b,
        since,
        filter,
      });
      bookingsVersion = ans.version;
      if (ans.changed && ans.data) {
        setRows(ans.data);
        setHasMore(ans.data.length >= 50);
      }
    } catch (err) {
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      } else {
        setError(String(err));
      }
    } finally {
      setLoading(false);
    }
  }

  async function fetchSettings(b: string, since?: number) {
    try {
      const ans = await api.view<AdminBookingSettingsData>('admin_booking_settings', {
        branch: b,
        since,
      });
      settingsVersion = ans.version;
      if (ans.changed && ans.data) {
        setSettings(ans.data);
      }
    } catch {
      // ignore
    }
  }

  function handleManualRefresh() {
    fetchBookings(props.branch(), bookingsVersion, currentQ || undefined);
    fetchSettings(props.branch(), settingsVersion);
  }

  async function handleLoadMore() {
    const currentRows = rows();
    const lastRow = currentRows[currentRows.length - 1];
    if (!lastRow) return;
    const q = searchTerm().trim();
    try {
      const nextRows = await api.page<AdminBookingRow>('admin_bookings', {
        branch: props.branch(),
        before: lastRow.id,
        limit: 50,
        filter: q ? { q } : undefined,
      });
      if (nextRows.length < 50) {
        setHasMore(false);
      }
      setRows([...currentRows, ...nextRows]);
    } catch (err) {
      if (err instanceof ApiError) {
        toast(errorText(err.code, err.message));
      }
    }
  }

  createEffect(() => {
    const isShown = props.shown();
    const b = props.branch();
    refreshTick();

    if (!isShown) return;

    untrack(() => {
      if (b !== currentBranch) {
        currentBranch = b;
        bookingsVersion = undefined;
        settingsVersion = undefined;
        setRows([]);
        setSettings(null);
        setHasMore(false);
        fetchBookings(b, undefined, currentQ || undefined);
        fetchSettings(b, undefined);
      } else {
        fetchBookings(b, bookingsVersion, currentQ || undefined);
        fetchSettings(b, settingsVersion);
      }
    });
  });

  createEffect(() => {
    const term = searchTerm().trim();
    if (!props.shown()) return;
    untrack(() => {
      if (term !== currentQ) {
        currentQ = term;
        bookingsVersion = undefined;
        fetchBookings(props.branch(), undefined, term || undefined);
      }
    });
  });

  function openConfirmDialog(row: AdminBookingRow) {
    const [expiry, setExpiry] = createSignal('');
    const [dialogError, setDialogError] = createSignal<string | null>(null);

    const confirmRate = async () => {
      try {
        await api.post('/api/bookings', {
          action: 'confirm',
          payload: {
            branchId: props.branch(),
            id: row.id,
            expiresAt: new Date(expiry()).toISOString(),
            expectedRate: row.counterRate ?? '',
          },
        });
        closeDialog();
        await fetchBookings(props.branch(), bookingsVersion, currentQ || undefined);
      } catch (err) {
        if (err instanceof ApiError) {
          setDialogError(errorText(err.code, err.message));
        } else {
          setDialogError(String(err));
        }
      }
    };

    openDialog(t('dialogConfirmTitle'), () => (
      <div>
        <div>
          {row.name} · {formatMoney(row.amount)} {row.currency}
        </div>
        <div>
          {t('rateToLock')}
          {row.counterRate ?? t('rateNotSet')}
          {t('principal')}
          {row.counterPrincipal ? formatMoney(row.counterPrincipal) : t('dash')}
          {t('thb')}
        </div>
        <label>
          {t('expiryField')}
          <input
            type="datetime-local"
            required
            value={expiry()}
            onInput={(e) => setExpiry(e.currentTarget.value)}
          />
        </label>
        <Show when={dialogError()}>
          <p class="error">{dialogError()}</p>
        </Show>
        <div class="actions">
          <button type="button" onClick={() => closeDialog()}>
            {t('back')}
          </button>
          <button type="button" onClick={confirmRate}>
            {t('confirmLockRate')}
          </button>
        </div>
      </div>
    ));
  }

  function openCancelDialog(row: AdminBookingRow) {
    const cancelBooking = async () => {
      try {
        await api.post('/api/bookings', {
          action: 'cancel',
          payload: {
            branchId: props.branch(),
            id: row.id,
          },
        });
        closeDialog();
        await fetchBookings(props.branch(), bookingsVersion, currentQ || undefined);
      } catch (err) {
        closeDialog();
        if (err instanceof ApiError) {
          toast(errorText(err.code, err.message));
        } else {
          toast(String(err));
        }
      }
    };

    openDialog(t('dialogCancelTitle'), () => (
      <div>
        <p>
          {t('cancelConfirmPrefix')}
          {row.id}
          {t('cancelConfirmSuffix')}
        </p>
        <div class="actions">
          <button type="button" onClick={() => closeDialog()}>
            {t('back')}
          </button>
          <button type="button" onClick={cancelBooking}>
            {t('confirmCancel')}
          </button>
        </div>
      </div>
    ));
  }

  async function openSettingsDialog() {
    let bName = props.branch();
    try {
      const res = await api.view<{ branches: Array<{ id: string; name: string }> }>('admin_branches');
      if (res.data?.branches) {
        const found = res.data.branches.find((b) => b.id === props.branch());
        if (found) bName = found.name;
      }
    } catch {
      // fallback to current branch id
    }

    const s = settings();
    const [mins, setMins] = createSignal(String(s ? s.rules.minutes : 30));
    const [enb, setEnb] = createSignal(s ? s.rules.enabled : true);
    const [dialogError, setDialogError] = createSignal<string | null>(null);

    const saveSettings = async () => {
      try {
        await api.post('/api/bookings', {
          action: 'settings',
          payload: {
            branchId: props.branch(),
            minutes: +mins(),
            enabled: enb(),
          },
        });
        closeDialog();
        await fetchSettings(props.branch(), settingsVersion);
      } catch (err) {
        if (err instanceof ApiError) {
          setDialogError(errorText(err.code, err.message));
        } else {
          setDialogError(String(err));
        }
      }
    };

    openDialog(`${t('settingsBranchPrefix')}${bName}`, () => (
      <div>
        <label>
          {t('settingsMinutes')}
          <input
            type="number"
            min="5"
            max="1440"
            required
            value={mins()}
            onInput={(e) => setMins(e.currentTarget.value)}
          />
        </label>
        <label>
          {t('settingsOpenWhenShift')}
          <input
            type="checkbox"
            checked={enb()}
            onChange={(e) => setEnb(e.currentTarget.checked)}
          />
        </label>
        <p>{t('settingsMemberNote')}</p>
        <Show when={dialogError()}>
          <p class="error">{dialogError()}</p>
        </Show>
        <div class="actions">
          <button type="button" onClick={saveSettings}>
            {t('save')}
          </button>
        </div>
      </div>
    ));
  }

  function openCreateDialog() {
    const requestId = crypto.randomUUID();
    const availableCurrencies = () =>
      (settings()?.counterRates ?? [])
        .map((r) => r.currency)
        .slice()
        .sort();

    const [name, setName] = createSignal('');
    const [phone, setPhone] = createSignal('');
    const [type, setType] = createSignal<'BUY' | 'SELL'>('BUY');
    const [currency, setCurrency] = createSignal(availableCurrencies()[0] ?? 'EUR');
    const [amount, setAmount] = createSignal('');
    const [customerId, setCustomerId] = createSignal('');
    const [dialogError, setDialogError] = createSignal<string | null>(null);

    const currentRate = () => {
      const cr = (settings()?.counterRates ?? []).find((r) => r.currency === currency());
      if (!cr) return '';
      return type() === 'BUY' ? cr.buy : cr.sell;
    };

    const handleCreateSubmit = async (e: Event) => {
      e.preventDefault();
      setDialogError(null);

      const trimmedName = name().trim();
      if (!trimmedName || name().length > 100) {
        setDialogError(t('errFillIn'));
        return;
      }

      const rawPhone = phone().trim();
      const digitCount = (rawPhone.match(/\d/g) || []).length;
      if (!/^[+0-9 ()-]{1,30}$/.test(rawPhone) || digitCount < 7) {
        setDialogError(t('errBadPhone'));
        return;
      }

      const rawAmount = amount().trim();
      const r = currentRate();
      if (!rawAmount || !/^\d+(\.\d+)?$/.test(rawAmount) || !/[1-9]/.test(rawAmount) || !r) {
        setDialogError(t('errBadAmountOrRate'));
        return;
      }

      const payload: Record<string, unknown> = {
        branchId: props.branch(),
        requestId,
        name: trimmedName,
        phone: rawPhone,
        type: type(),
        currency: currency(),
        amount: rawAmount,
        expectedRate: r,
      };
      const cId = customerId().trim();
      if (cId) {
        payload.customerId = cId;
      }

      try {
        await api.post('/api/bookings', {
          action: 'create',
          payload,
        });
        closeDialog();
        await fetchBookings(props.branch(), bookingsVersion, currentQ || undefined);
      } catch (err) {
        if (err instanceof ApiError) {
          setDialogError(errorText(err.code, err.message));
        } else {
          setDialogError(String(err));
        }
      }
    };

    openDialog(t('createButton'), () => (
      <form onSubmit={handleCreateSubmit}>
        <label>
          {t('fieldName')}
          <input
            type="text"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
        </label>
        <label>
          {t('fieldPhone')}
          <input
            type="tel"
            value={phone()}
            onInput={(e) => setPhone(e.currentTarget.value)}
          />
        </label>
        <label>
          {t('fieldType')}
          <select
            value={type()}
            onChange={(e) => setType(e.currentTarget.value as 'BUY' | 'SELL')}
          >
            <option value="BUY">{t('typeBuy')}</option>
            <option value="SELL">{t('typeSell')}</option>
          </select>
        </label>
        <label>
          {t('fieldCurrency')}
          <select
            value={currency()}
            onChange={(e) => setCurrency(e.currentTarget.value)}
          >
            <For each={availableCurrencies()}>
              {(c) => <option value={c}>{c}</option>}
            </For>
          </select>
        </label>
        <label>
          {t('fieldAmount')}
          <input
            type="text"
            value={amount()}
            onInput={(e) => setAmount(e.currentTarget.value)}
          />
        </label>
        <label>
          {t('fieldRate')}
          <input type="text" readOnly value={currentRate()} />
        </label>
        <label>
          {t('fieldCustomerId')}
          <input
            type="text"
            value={customerId()}
            onInput={(e) => setCustomerId(e.currentTarget.value)}
          />
        </label>
        <Show when={dialogError()}>
          <p class="error">{dialogError()}</p>
        </Show>
        <div class="actions">
          <button type="submit">{t('createButton')}</button>
        </div>
      </form>
    ));
  }

  return (
    <div class="admin-bookings">
      <div class="toolbar">
        <Show when={settings()?.publicUrl}>
          <a href={settings()!.publicUrl} target="_blank" rel="noopener">
            {t('openPublicWeb')}
          </a>
        </Show>
        <button type="button" onClick={handleManualRefresh}>
          {t('refreshBookings')}
        </button>
        <button type="button" onClick={openSettingsDialog}>
          {t('bookingSettings')}
        </button>
        <button type="button" onClick={openCreateDialog}>
          {t('createButtonWithPlus')}
        </button>
        <label>
          {t('searchBookings')}
          <input
            type="text"
            placeholder={t('searchPlaceholder')}
            maxlength={100}
            value={searchTerm()}
            onInput={(e) => setSearchTerm(e.currentTarget.value)}
          />
        </label>
      </div>

      <div class="info-line">
        {t('infoPrefix')}
        {settings() ? settings()!.rules.minutes : 30}
        {t('infoMinutes')}
        {settings()?.rules.enabled ? t('infoOpen') : t('infoClosed')}
        {t('infoMembers')}
        {settings() ? settings()!.memberCount : 0}
        {t('infoPersons')}
      </div>

      <Show when={loading()}>
        <p>{t('loading')}</p>
      </Show>
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <Show when={!loading() && !error() && rows().length === 0}>
        <p>{t('emptySearch')}</p>
      </Show>
      <Show when={!loading() && !error() && rows().length > 0}>
        <table>
          <thead>
            <tr>
              <th>{t('thBookingCustomer')}</th>
              <th>{t('thItem')}</th>
              <th>{t('thRate')}</th>
              <th>{t('thExpiry')}</th>
              <th>{t('thStatus')}</th>
              <th>{t('thActions')}</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(row) => (
                <tr>
                  <td>
                    <div>{row.id}</div>
                    <div>
                      {row.name} · {row.phone}
                      <Show when={row.member}>
                        {' '}<span class="pill">{t('memberPill')}</span>
                      </Show>
                    </div>
                  </td>
                  <td>
                    {row.type === 'BUY' ? t('typeBuy') : t('typeSell')}{' '}
                    {formatMoney(row.amount)} {row.currency}
                  </td>
                  <td>{row.rate}</td>
                  <td>{formatExpiry(row.expiresAt, t, locale())}</td>
                  <td>{getStatusText(row.status, t)}</td>
                  <td>
                    <Show when={row.status === 'PENDING'}>
                      <button type="button" onClick={() => openConfirmDialog(row)}>
                        {t('confirmRateAndTime')}
                      </button>
                      <button type="button" onClick={() => openCancelDialog(row)}>
                        {t('cancel')}
                      </button>
                    </Show>
                    <Show when={row.status === 'CONFIRMED'}>
                      <a
                        href={`/desk?branch=${props.branch()}&booking=${row.id}`}
                        target="_blank"
                        rel="noopener"
                      >
                        {t('deskHandOff')}
                      </a>
                      <button type="button" onClick={() => openCancelDialog(row)}>
                        {t('cancel')}
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <Show when={hasMore()}>
          <div class="load-more">
            <button type="button" onClick={handleLoadMore}>
              {t('loadMore')}
            </button>
          </div>
        </Show>
      </Show>
    </div>
  );
}
