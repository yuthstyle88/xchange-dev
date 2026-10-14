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
import { errorText, screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { refreshTick } from '../parts/refresh';
import { Card, CardHead, CardBody, CardGrid } from '../../../shared/ui/Card';
import { StatCard } from '../../../shared/ui/StatCard';
import { toast } from '../parts/toast';
import { CashFlows } from './CashFlows';
import { openShiftDetail } from './ShiftDetail';
import * as table from './i18n';
import './cash.css';

const t = screenT('admin.cash', table);

function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
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

  function toWordedError(err: unknown): string {
    const generic = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    if (err instanceof ApiError) {
      return errorText(err.code, generic);
    }
    return generic;
  }

  async function readCash(resetVersion = false, all = showAll(), isReadAll = false) {
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
        if (!isReadAll) {
          setCashError(null);
        }
      }
    } catch (err: unknown) {
      setCashError(toWordedError(err));
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
    } catch (err: unknown) {
      setMovements([]);
      setCashError(toWordedError(err));
    }
  }

  async function readShifts() {
    try {
      const rows = await api.page<AdminShiftRow>('admin_shifts', {
        branch: props.branch(),
      });
      setShifts(rows);
      setHasMoreShifts(rows.length > 0);
    } catch (err: unknown) {
      setShifts([]);
      setHasMoreShifts(false);
      setCashError(toWordedError(err));
    }
  }

  async function readAll(resetVersion = true) {
    setCashError(null);
    await Promise.all([readCash(resetVersion, showAll(), true), readMovements(), readShifts()]);
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
          <button type="button" class="primary" onClick={closeDialog}>
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
                class="secondary"
                onClick={handleExampleDialog}
                disabled={isWriteDisabled()}
              >
                {t('btnViewExample')}
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
                        <td class={+row.expected < 0 ? 'num negative' : 'num'}>
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

      {/* CA-9 #625: counts waiting and transfers on CA-8, each by its permission */}
      <CashFlows branch={props.branch} shown={props.shown} shifts={shifts} />

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
