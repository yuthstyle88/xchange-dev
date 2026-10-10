import {
  createEffect,
  createSignal,
  For,
  Show,
  untrack,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminCashData, AdminCashFilter } from '../../../shared/api/generated/admin_cash';
import type { AdminCashMovementRow } from '../../../shared/api/generated/admin_cash_movements';
import type { AdminShiftRow } from '../../../shared/api/generated/admin_shifts';
import { errorText, lang, screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { refreshTick } from '../parts/refresh';
import { Card, CardHead, CardBody, CardGrid } from '../../../shared/ui/Card';
import { StatCard } from '../../../shared/ui/StatCard';
import { toast } from '../parts/toast';
import { openShiftDetail } from './ShiftDetail';
import * as table from './i18n';
import './cash.css';

const t = screenT('admin.cash', table);

function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
}

function getCurrencyName(code: string): string {
  try {
    const dn = new Intl.DisplayNames([lang()], { type: 'currency' });
    return dn.of(code) ?? code;
  } catch {
    return code;
  }
}

interface OpeningRow {
  code: string;
  amount: string;
}

export function Cash(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [cash, setCash] = createSignal<AdminCashData | null>(null);
  const [cashError, setCashError] = createSignal<string | null>(null);
  const [movements, setMovements] = createSignal<AdminCashMovementRow[]>([]);
  const [shifts, setShifts] = createSignal<AdminShiftRow[]>([]);
  const [showAll, setShowAll] = createSignal<boolean>(false);
  const [loadingOlderShifts, setLoadingOlderShifts] = createSignal<boolean>(false);
  const [olderError, setOlderError] = createSignal<string | null>(null);
  const [hasMoreShifts, setHasMoreShifts] = createSignal<boolean>(true);

  let lastBranch = '';
  let lastTick: number | undefined = undefined;
  let currentCashVersion: number | undefined = undefined;

  async function readCash(resetVersion = false, all = showAll()) {
    const curBranch = props.branch();
    if (resetVersion || curBranch !== lastBranch) {
      lastBranch = curBranch;
      currentCashVersion = undefined;
    }
    try {
      const res = await api.view<AdminCashData, AdminCashFilter>('admin_cash', {
        branch: curBranch,
        since: currentCashVersion,
        filter: { all },
      });
      currentCashVersion = res.version;
      if (res.changed && res.data) {
        setCash(res.data);
        setCashError(null);
      }
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setCashError(errorText(err.code, err.message));
      } else {
        setCashError(String(err));
      }
      setCash(null);
    }
  }

  async function readMovements() {
    try {
      const rows = await api.page<AdminCashMovementRow>('admin_cash_movements', {
        branch: props.branch(),
        limit: 100,
      });
      setMovements(rows);
    } catch {
      setMovements([]);
    }
  }

  async function readShifts() {
    try {
      const rows = await api.page<AdminShiftRow>('admin_shifts', {
        branch: props.branch(),
      });
      setShifts(rows);
      setHasMoreShifts(rows.length > 0);
    } catch {
      setShifts([]);
    }
  }

  async function readAll(resetVersion = true) {
    await Promise.all([readCash(resetVersion, showAll()), readMovements(), readShifts()]);
  }

  createEffect(() => {
    if (!props.shown()) return;
    const currentBranch = props.branch();
    const currentTick = refreshTick();

    untrack(() => {
      if (lastBranch !== currentBranch) {
        lastBranch = currentBranch;
        lastTick = currentTick;
        readAll(true);
      } else if (lastTick !== currentTick) {
        lastTick = currentTick;
        readCash(false, showAll());
      }
    });
  });

  function handleToggleShowAll(e: Event & { currentTarget: HTMLInputElement }) {
    const checked = e.currentTarget.checked;
    setShowAll(checked);
    readCash(true, checked);
  }

  async function handleLoadOlderShifts() {
    if (loadingOlderShifts()) return;
    const current = shifts();
    const last = current[current.length - 1];
    if (!last) return;
    setLoadingOlderShifts(true);
    setOlderError(null);
    try {
      const older = await api.page<AdminShiftRow>('admin_shifts', {
        branch: props.branch(),
        before: last.id,
        limit: 50,
      });
      if (older.length === 0) {
        setHasMoreShifts(false);
      } else {
        setShifts((prev) => [...prev, ...older]);
      }
    } catch (err: unknown) {
      const msg = err instanceof ApiError ? errorText(err.code, err.message) : String(err);
      setOlderError(msg);
      toast(msg);
    } finally {
      setLoadingOlderShifts(false);
    }
  }

  const isWriteDisabled = () => pendingWrite.busy() || cashError() !== null;
  const hasOpenShift = () => Boolean(cash()?.shift);

  function shiftDiffText(s: AdminShiftRow): string {
    if (s.status === 'OPEN') {
      return '—';
    }
    const diffs = s.difference ?? [];
    if (diffs.length === 0) {
      return t('diffMatched');
    }
    return diffs.map((d) => `${d.code} ${formatMoney(d.amount)}`).join(' · ');
  }

  // ── Dialogs ──────────────────────────────────────────────────────────────────

  function handleOpenShiftDialog() {
    const data = cash();
    const lastCounted = data?.lastCounted;
    const hasPrefill = Boolean(lastCounted && lastCounted.length > 0);

    const initialRows: OpeningRow[] = hasPrefill
      ? lastCounted!.map((c) => ({ code: c.code, amount: c.amount }))
      : [{ code: 'THB', amount: '0' }];

    const [rows, setRows] = createSignal<OpeningRow[]>(initialRows);
    const [searchTerm, setSearchTerm] = createSignal<string>('');

    const availableRegister = () => data?.currencies ?? [];

    const searchMatches = () => {
      const term = searchTerm().trim().toLowerCase();
      if (term === '') return [];
      const currentCodes = rows().map((r) => r.code);
      return availableRegister().filter((code) => {
        if (currentCodes.includes(code)) return false;
        const codeMatch = code.toLowerCase().includes(term);
        const nameMatch = getCurrencyName(code).toLowerCase().includes(term);
        return codeMatch || nameMatch;
      });
    };

    function addCurrency(code: string) {
      setRows((prev) => [...prev, { code, amount: '' }]);
      setSearchTerm('');
    }

    function removeCurrency(code: string) {
      setRows((prev) => prev.filter((r) => r.code !== code));
    }

    function updateAmount(code: string, amount: string) {
      setRows((prev) =>
        prev.map((r) => (r.code === code ? { ...r, amount } : r)),
      );
    }

    function onSearchKeyDown(e: KeyboardEvent) {
      if (e.key === 'Enter') {
        e.preventDefault();
        const matches = searchMatches();
        if (matches.length === 1) {
          addCurrency(matches[0]!);
        }
      }
    }

    const onSubmit = (e: Event) => {
      e.preventDefault();
      const opening: Record<string, string> = {};
      const rowMap = new Map(rows().map((r) => [r.code, r.amount]));
      for (const code of availableRegister()) {
        if (rowMap.has(code)) {
          opening[code] = rowMap.get(code) || '0';
        } else {
          opening[code] = '0';
        }
      }

      pendingWrite.run(
        api,
        props.branch(),
        'adminOpenShift',
        { opening },
        () => {
          lastTick = refreshTick();
          readAll(true);
        },
      );
    };

    openDialog(t('btnOpenShift'), () => (
      <form onSubmit={onSubmit} class="cash-open-dialog">
        <p class="dialog-note">
          {hasPrefill ? t('openNotePrefill') : t('openNoteClean')}
        </p>

        <div class="search-currency-section">
          <label for="open-search">{t('openSearchLabel')}</label>
          <input
            id="open-search"
            type="text"
            placeholder={t('openSearchPlaceholder')}
            value={searchTerm()}
            onInput={(e) => setSearchTerm(e.currentTarget.value)}
            onKeyDown={onSearchKeyDown}
          />
          <Show when={searchTerm().trim() !== '' && searchMatches().length === 0}>
            <p class="search-empty">{t('openSearchNotFound')}</p>
          </Show>
          <Show when={searchMatches().length > 0}>
            <div class="search-results">
              <For each={searchMatches()}>
                {(code) => (
                  <button
                    type="button"
                    class="add-currency-btn"
                    onClick={() => addCurrency(code)}
                  >
                    + {code} {getCurrencyName(code)}
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>

        <div class="currency-rows">
          <For each={rows()}>
            {(row) => (
              <div class="currency-row">
                <Show
                  when={row.code === 'THB'}
                  fallback={
                    <label for={`open-${row.code}`}>
                      {row.code} · {getCurrencyName(row.code)}
                    </label>
                  }
                >
                  <label for="open-THB">{t('thbOpeningLabel')}</label>
                </Show>
                <input
                  id={`open-${row.code}`}
                  type="text"
                  value={row.amount}
                  onInput={(e) => updateAmount(row.code, e.currentTarget.value)}
                  required={row.code === 'THB'}
                />
                <Show when={row.code !== 'THB'}>
                  <button
                    type="button"
                    class="remove-currency-btn"
                    aria-label={t('deleteCurrencyAria', { code: row.code })}
                    onClick={() => removeCurrency(row.code)}
                  >
                    {t('remove')}
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>

        <div class="dialog-actions">
          <button type="button" onClick={closeDialog}>
            {t('cancel')}
          </button>
          <button type="submit">{t('confirmOpenShift')}</button>
        </div>
      </form>
    ));
  }

  function handleMoveStockDialog() {
    const data = cash();
    let formEl: HTMLFormElement | undefined;
    const [reasonError, setReasonError] = createSignal<string | null>(null);

    const onSubmit = (e: Event) => {
      e.preventDefault();
      if (!formEl || !data?.shift) return;
      setReasonError(null);
      const fd = new FormData(formEl);
      const reason = String(fd.get('reason') ?? '').trim();
      if (!reason) {
        setReasonError(t('specifyReason'));
        return;
      }

      pendingWrite.run(
        api,
        props.branch(),
        'adminMoveStock',
        {
          shiftId: data.shift.id,
          currency: String(fd.get('currency') ?? 'THB'),
          direction: String(fd.get('direction') ?? 'IN'),
          amount: String(fd.get('amount') ?? '0'),
          reason,
        },
        () => {
          lastTick = refreshTick();
          readAll(true);
        },
      );
    };

    openDialog(t('movementDialogTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit} class="cash-movement-dialog">
        <p class="dialog-note">{t('movementDialogNote')}</p>

        <div>
          <label for="move-currency">{t('headerCurrency')}</label>
          <select id="move-currency" name="currency">
            <For each={data?.currencies ?? []}>{(c) => <option value={c}>{c}</option>}</For>
          </select>
        </div>

        <div>
          <label for="move-direction">{t('movementTypeLabel')}</label>
          <select id="move-direction" name="direction">
            <option value="IN">{t('dirIn')}</option>
            <option value="OUT">{t('dirOut')}</option>
          </select>
        </div>

        <div>
          <label for="move-amount">{t('headerAmount')}</label>
          <input id="move-amount" name="amount" type="text" required />
        </div>

        <div>
          <label for="move-reason">{t('movementReasonLabel')}</label>
          <input
            id="move-reason"
            name="reason"
            type="text"
            maxLength={200}
            placeholder={t('movementReasonPlaceholder')}
          />
        </div>

        <Show when={reasonError()}>
          <p role="alert" class="error-message">
            {reasonError()}
          </p>
        </Show>

        <div class="dialog-actions">
          <button type="submit">{t('saveMovement')}</button>
        </div>
      </form>
    ));
  }

  function handleCloseShiftDialog() {
    const data = cash();
    if (!data?.shift) return;

    const [counts, setCounts] = createSignal<Record<string, string>>({});
    const [note, setNote] = createSignal<string>('');

    const expectedMap = new Map<string, string>();
    for (const r of data.stock) {
      expectedMap.set(r.code, r.expected);
    }

    function updateCount(code: string, val: string) {
      setCounts((prev) => ({ ...prev, [code]: val }));
    }

    const onSubmit = (e: Event) => {
      e.preventDefault();
      const counted: Record<string, string> = {};
      const currentCounts = counts();
      for (const code of data.currencies) {
        counted[code] = currentCounts[code] ?? '';
      }

      const expected: Record<string, string> = {};
      for (const r of data.stock) {
        expected[r.code] = r.expected;
      }

      pendingWrite.run(
        api,
        props.branch(),
        'adminCloseShift',
        {
          shiftId: data.shift!.id,
          expected,
          counted,
          note: note().trim(),
        },
        () => {
          lastTick = refreshTick();
          readAll(true);
        },
      );
    };

    openDialog(t('btnCloseShift'), () => (
      <form onSubmit={onSubmit} class="cash-close-dialog">
        <div class="close-currencies-list">
          <For each={data.currencies}>
            {(code) => {
              const exp = () => expectedMap.get(code) ?? '0.00';
              return (
                <div class="close-currency-row">
                  <Show
                    when={code === 'THB'}
                    fallback={<label for={`close-${code}`}>{code}</label>}
                  >
                    <label for="close-THB">{t('closeThbLabel')}</label>
                  </Show>
                  <span class="expected-figure">
                    {t('closeAsExpected')} {formatMoney(exp())}
                  </span>
                  <input
                    id={`close-${code}`}
                    type="text"
                    value={counts()[code] ?? ''}
                    onInput={(e) => updateCount(code, e.currentTarget.value)}
                    required
                  />
                  <span class="counted-echo">{counts()[code] ?? ''}</span>
                </div>
              );
            }}
          </For>
        </div>

        <div>
          <label for="close-note">{t('closeNoteLabel')}</label>
          <input
            id="close-note"
            type="text"
            placeholder={t('closeNotePlaceholder')}
            value={note()}
            onInput={(e) => setNote(e.currentTarget.value)}
          />
        </div>

        <div class="dialog-actions">
          <button type="button" onClick={closeDialog}>
            {t('backToCount')}
          </button>
          <button type="submit">{t('confirmCloseShift')}</button>
        </div>
      </form>
    ));
  }

  function handleExampleDialog() {
    openDialog(t('exampleTitle'), () => (
      <div class="cash-example-dialog">
        <p class="dialog-note">{t('exampleSubtitle')}</p>

        <div class="example-grid">
          <div>{t('statOpening')}</div>
          <div>500,000.00 THB</div>

          <div>{t('exampleCustomerSells')}</div>
          <div>10,000.00 USD</div>

          <div>{t('exampleRefRate')}</div>
          <div>32.50 → 325,000.00 THB</div>

          <div>{t('exampleShopRate')}</div>
          <div>31.59 → 315,900.00 THB</div>

          <div>{t('exampleSpread')}</div>
          <div>9,100.00 THB · 2.80%</div>

          <div>{t('statRemaining')}</div>
          <div>184,100.00 THB</div>

          <div>{t('exampleUsdStock')}</div>
        </div>

        <p class="example-explanation">{t('exampleExplanation')}</p>

        <div class="dialog-actions">
          <button type="button" onClick={closeDialog}>
            {t('understood')}
          </button>
        </div>
      </div>
    ));
  }

  return (
    <div class="admin-cash-screen">
      <Show when={cashError()}>
        <div class="error-banner" role="alert">
          {cashError()}
        </div>
      </Show>

      {/* Banner */}
      <Show
        when={cash()?.shift}
        fallback={
          <div class="stock-banner">
            <strong>{t('noOpenShift')}</strong>
            <p>{t('noOpenShiftSubtitle')}</p>
          </div>
        }
      >
        <div class="stock-banner">
          <strong>
            {t('openShiftBanner', { id: cash()!.shift!.id })}
          </strong>
          <p>
            {t('openShiftBannerSub', {
              when: when(cash()!.shift!.openedAt),
              staff: cash()!.shift!.staff,
            })}
          </p>
        </div>
      </Show>

      {/* 4 Stat Cards */}
      <CardGrid>
        <StatCard
          label={t('statOpening')}
          value={cash()?.stats ? cash()!.stats!.opening : '—'}
          unit="THB"
          note={t('statOpeningNote')}
        />
        <StatCard
          label={t('statIncoming')}
          value={cash()?.stats ? cash()!.stats!.incoming : '—'}
          unit="THB"
          note={t('statIncomingNote')}
        />
        <StatCard
          label={t('statOutgoing')}
          value={cash()?.stats ? cash()!.stats!.outgoing : '—'}
          unit="THB"
          note={t('statOutgoingNote')}
        />
        <StatCard
          label={t('statRemaining')}
          value={cash()?.stats ? cash()!.stats!.remaining : '—'}
          unit="THB"
          note={t('statRemainingNote')}
        />
      </CardGrid>

      {/* Reference Summary */}
      <div class="stock-banner">
        <strong>
          {cash()?.shift && cash()?.reference && cash()!.reference.withReference > 0
            ? t('refSummaryWithRates', {
                spread: formatMoney(cash()!.reference.spread),
                percent: cash()!.reference.percent,
              })
            : t('refSummaryNoRates')}
        </strong>
        <p>
          {t('refSummaryCounts', {
            withRef: cash()?.reference.withReference ?? 0,
            withoutRef: cash()?.reference.withoutReference ?? 0,
          })}
        </p>
      </div>

      {/* Stock Card and Actions */}
      <Card class="stock-card">
        <CardHead
          eyebrow={t('stockEyebrow')}
          title={t('stockTitle')}
          aside={
            <div class="cash-actions">
              <button
                type="button"
                class="primary"
                onClick={handleOpenShiftDialog}
                disabled={isWriteDisabled() || hasOpenShift()}
              >
                {t('btnOpenShift')}
              </button>
              <button
                type="button"
                class="secondary"
                onClick={handleExampleDialog}
                disabled={isWriteDisabled()}
              >
                {t('btnViewExample')}
              </button>
              <button
                type="button"
                class="secondary"
                onClick={handleMoveStockDialog}
                disabled={isWriteDisabled() || !hasOpenShift()}
              >
                {t('btnMoveStock')}
              </button>
              <button
                type="button"
                class="secondary"
                onClick={handleCloseShiftDialog}
                disabled={isWriteDisabled() || !hasOpenShift()}
              >
                {t('btnCloseShift')}
              </button>
            </div>
          }
        />
        <CardBody flush>
          <label class="stock-filter">
            <input
              type="checkbox"
              checked={showAll()}
              onChange={handleToggleShowAll}
            />{' '}
            {t('filterAll')} <span>{t('filterAllNote')}</span>
          </label>

          <Show
            when={hasOpenShift()}
            fallback={
              <div class="empty">
                <strong>{t('stockNote1')}</strong>
                <p>{t('stockNote2')}</p>
              </div>
            }
          >
            <div class="table-scroll">
              <table class="stock-table">
                <thead>
                  <tr>
                    <th>{t('headerCurrency')}</th>
                    <th class="num">{t('headerOpening')}</th>
                    <th class="num">{t('headerNet')}</th>
                    <th class="num">{t('headerExpected')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={cash()?.stock ?? []}>
                    {(row) => (
                      <tr>
                        <td>
                          <strong>{row.code}</strong>
                        </td>
                        <td class="num">{formatMoney(row.opening)}</td>
                        <td class="num">{formatMoney(row.net)}</td>
                        <td class="num">
                          <strong>{formatMoney(row.expected)}</strong>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </CardBody>
      </Card>

      {/* Footnote */}
      <p class="footnote">{t('footnote')}</p>

      {/* History Details */}
      <details class="workspace-history">
        <summary>{t('historySummary')}</summary>

        <Card>
          <CardHead title={t('historyMovementsTitle')} />
          <Show
            when={movements().length > 0}
            fallback={
              <div class="empty">
                <strong>{t('emptyMovements')}</strong>
              </div>
            }
          >
            <div class="table-scroll">
              <table class="movements-table">
                <thead>
                  <tr>
                    <th>{t('headerDateTime')}</th>
                    <th>{t('headerShift')}</th>
                    <th>{t('headerItem')}</th>
                    <th class="num">{t('headerAmount')}</th>
                    <th>{t('headerReason')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={movements()}>
                    {(m) => (
                      <tr>
                        <td>{when(m.created)}</td>
                        <td>{m.shiftId}</td>
                        <td>
                          <span class={m.direction === 'OUT' ? 'pill buy' : 'pill'}>
                            {m.direction === 'IN' ? t('dirIn') : t('dirOut')}
                          </span>
                        </td>
                        <td class="num">
                          {formatMoney(m.amount)} {m.currency}
                        </td>
                        <td>{m.reason}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </Card>

        <Card>
          <CardHead title={t('historyShiftsTitle')} />
          <Show
            when={shifts().length > 0}
            fallback={
              <div class="empty">
                <strong>{t('emptyShifts')}</strong>
              </div>
            }
          >
            <div class="table-scroll">
              <table class="shifts-table">
                <thead>
                  <tr>
                    <th>{t('headerShift')}</th>
                    <th>{t('headerOpenedAt')}</th>
                    <th>{t('headerOpenedBy')}</th>
                    <th>{t('headerStatus')}</th>
                    <th>{t('headerDifferenceClosed')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={shifts()}>
                    {(s) => (
                      <tr>
                        <td>
                          <button
                            type="button"
                            class="ref"
                            onClick={() => openShiftDetail(s, props.branch())}
                          >
                            {s.id}
                          </button>
                        </td>
                        <td>{when(s.openedAt)}</td>
                        <td>{s.staff}</td>
                        <td>
                          <span class={s.status === 'CLOSED' ? 'pill neutral' : 'pill'}>
                            {s.status === 'OPEN' ? t('statusOpen') : t('statusClosed')}
                          </span>
                        </td>
                        <td>{shiftDiffText(s)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>

            <Show when={hasMoreShifts()}>
              <div class="load-older-shifts">
                <button
                  type="button"
                  onClick={handleLoadOlderShifts}
                  disabled={loadingOlderShifts()}
                >
                  {t('loadOlder')}
                </button>
                <Show when={olderError()}>
                  <p role="alert" class="error-message">
                    {olderError()}
                  </p>
                </Show>
              </div>
            </Show>
          </Show>
        </Card>
      </details>
    </div>
  );
}
