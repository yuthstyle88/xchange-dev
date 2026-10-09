// V2-overview (#461): overview screen component (ticket §7–§10 B1–B6; design §12.1, §12.2, §12.6, §12.14, W7, W10)

import {
  createEffect,
  createSignal,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminOverviewData } from '../../../shared/api/generated/admin_overview';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { openMenu } from '../parts/hash';
import { NewTransactionButton } from '../parts/NewTransactionButton';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import { TransactionDetail } from '../parts/TransactionDetail';
import { TransactionTable } from '../parts/TransactionTable';
import { downloadCsv, walkPages } from '../parts/csv';
import { StatCard } from '../parts/StatCard';
import { en, th } from './i18n';

const CSV_HEADERS = [
  'Branch ID',
  'Branch',
  'Mode',
  'Document Ref',
  'Date',
  'Type',
  'Currency',
  'Amount',
  'Rate',
  'Principal THB',
  'Fee THB',
  'Bank Fee THB',
  'Settlement THB',
  'Method',
  'Status',
  'Shift ID',
  'Reference Rate',
  'Reference Value THB',
  'Reference Spread THB',
  'Reference Spread %',
];

export function Overview(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.overview', { th, en });
  const api = new ApiClient();

  const [data, setData] = createSignal<AdminOverviewData | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  let heldVersion: number | undefined = undefined;
  let activeBranch = '';
  let activeFrom = '';
  let activeTo = '';
  let currentFetchId = 0;

  async function loadData(opts: { isRefresh?: boolean } = {}) {
    const branch = props.branch();
    const period = props.period();

    // Never send an invalid period whose from is after to
    if (period.from && period.to && period.from > period.to) {
      return;
    }

    const fetchId = ++currentFetchId;
    const since = opts.isRefresh ? heldVersion : undefined;
    const filter: Record<string, string> = {};
    if (period.from) filter.from = period.from;
    if (period.to) filter.to = period.to;

    try {
      const answer = await api.view<AdminOverviewData>('admin_overview', {
        branch,
        since,
        filter,
      });

      if (fetchId !== currentFetchId) return;

      setError(null);
      heldVersion = answer.version;
      if (answer.changed && answer.data) {
        setData(answer.data);
      }
    } catch (err: unknown) {
      if (fetchId !== currentFetchId) return;
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      } else {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      }
    }
  }

  createEffect(() => {
    if (!props.shown()) return;

    const branch = props.branch();
    const period = props.period();

    const branchChanged = branch !== activeBranch;
    const periodChanged =
      period.from !== activeFrom || period.to !== activeTo;

    if (branchChanged || periodChanged) {
      activeBranch = branch;
      activeFrom = period.from;
      activeTo = period.to;
      heldVersion = undefined;
      loadData({ isRefresh: false });
    }
  });

  let lastSeenTick = refreshTick();
  createEffect(() => {
    const tick = refreshTick();
    if (tick !== lastSeenTick) {
      lastSeenTick = tick;
      if (props.shown()) {
        loadData({ isRefresh: true });
      }
    }
  });

  function handleOpenDetail(id: string) {
    const row = data()?.latest.find((r) => r.id === id);
    if (!row) return;
    openDialog(t('dialogTitle'), () => (
      <TransactionDetail
        api={api}
        branch={props.branch()}
        row={row}
        onClose={closeDialog}
      />
    ));
  }

  function handleCheckPending() {
    openMenu('transactions', {
      status: 'PENDING',
      from: '',
      to: '',
      q: '',
      type: '',
    });
  }

  async function handleExportCsv() {
    const branch = props.branch();
    const period = props.period();
    const filter: Record<string, string> = {};
    if (period.from) filter.from = period.from;
    if (period.to) filter.to = period.to;

    try {
      const allRows = await walkPages<TransactionRow>(
        api,
        'admin_transactions',
        {
          branch,
          filter,
        },
      );

      if (allRows.length === 0) {
        toast(t('noTransactionsToExport'));
        return;
      }

      let branchName = branch;
      try {
        const res = await api.view<{
          branches: { id: string; name: string }[];
        }>('admin_branches');
        const b = res.data?.branches?.find((x) => x.id === branch);
        if (b) branchName = b.name;
      } catch {
        // fallback
      }

      const todayBangkok = new Intl.DateTimeFormat('en-CA', {
        timeZone: 'Asia/Bangkok',
      }).format(new Date());
      const safeBranch = branchName.replace(/[^a-zA-Z0-9-]/g, '_');
      const filename = `XCHANGE-LIVE-${safeBranch}-${todayBangkok}.csv`;

      const csvRows = allRows.map((r) => [
        branch,
        branchName,
        'LIVE',
        r.id,
        r.date,
        r.type,
        r.currency,
        r.amount,
        r.rate,
        r.principal,
        r.fee,
        r.bankFee,
        r.settlement,
        r.method,
        r.status,
        r.shiftId ?? '',
        r.referenceRate ?? '',
        r.referenceValue ?? '',
        r.referenceSpread ?? '',
        r.referencePercent ?? '',
      ]);

      downloadCsv(filename, CSV_HEADERS, csvRows);
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      }
    }
  }

  const netDisplay = () => {
    const net = data()?.totals.net ?? '0.00';
    if (!net.startsWith('-') && net !== '0.00' && net !== '0') {
      return `+${formatMoney(net)}`;
    }
    return formatMoney(net);
  };

  return (
    <div class="overview-screen">
      <Show when={error()}>
        {(err) => <div class="error-banner">{err()}</div>}
      </Show>

      {/* Top action toolbar */}
      <div class="overview-toolbar">
        <div class="toolbar-actions">
          <NewTransactionButton api={api} branch={props.branch()} />
          <button
            type="button"
            class="btn-export"
            onClick={handleExportCsv}
          >
            {t('exportCsv')}
          </button>
        </div>
      </div>

      {/* Status Strip (B5) */}
      <div class="status-strip">
        <div class="strip-header">
          <span class="strip-title">{t('statusStripTitle')}</span>
        </div>
        <div class="strip-body">
          <Show
            when={!error()}
            fallback={
              <div class="strip-section status-error-section">
                <div class="strip-state-text">{t('statusError')}</div>
                <div class="strip-hint">{t('refreshHint')}</div>
              </div>
            }
          >
            <Show
              when={data()?.status.shiftOpen}
              fallback={
                <div class="strip-section status-closed-section">
                  <div class="strip-state-text">{t('shiftClosed')}</div>
                  <div class="strip-hint">{t('openShiftHint')}</div>
                  <a href="#cash">{t('viewCashLink')}</a>
                </div>
              }
            >
              <div class="strip-section status-open-section">
                <div class="strip-state-text">{t('shiftOpen')}</div>
                <Show when={data()?.status.openedBy}>
                  {(opener) => (
                    <div class="strip-opener">
                      {t('openedByLabel', { staff: opener() })}
                    </div>
                  )}
                </Show>
              </div>
            </Show>
          </Show>

          {/* Pending items check */}
          <div class="strip-section pending-section">
            <div class="pending-label">{t('pendingTotalLabel')}</div>
            <div class="pending-count">
              <Show
                when={!error()}
                fallback={<span class="pending-dash">—</span>}
              >
                {t('pendingItemCount', {
                  count: data()?.status.pendingCount ?? 0,
                })}
              </Show>
            </div>
            <div class="pending-note">{t('pendingNote')}</div>
            <button
              type="button"
              disabled={error() !== null}
              onClick={handleCheckPending}
            >
              {t('checkPendingBtn')}
            </button>
          </div>
        </div>
      </div>

      {/* Four Stat Cards (B1) */}
      <div class="overview-cards">
        <StatCard
          label={t('totalVolume')}
          value={data()?.totals.volume ?? '0.00'}
          unit="THB"
          note={t('totalVolumeNote')}
        />
        <StatCard
          label={t('buyCardLabel')}
          value={data()?.totals.buyPrincipal ?? '0.00'}
          unit="THB"
          note={t('buyCardNote', {
            count: data()?.totals.buyCount ?? 0,
          })}
        />
        <StatCard
          label={t('sellCardLabel')}
          value={data()?.totals.sellPrincipal ?? '0.00'}
          unit="THB"
          note={t('sellCardNote', {
            count: data()?.totals.sellCount ?? 0,
          })}
        />
        <StatCard
          label={t('completedTransactions')}
          value={String(data()?.totals.completedCount ?? 0)}
          unit={t('completedUnit')}
          note={t('completedNote', {
            cancelled: data()?.totals.cancelledCount ?? 0,
            pending: data()?.totals.pendingCount ?? 0,
          })}
        />
      </div>

      {/* Activity and Flow row (B2 & B3) */}
      <div class="overview-split">
        {/* Activity BUY/SELL bars (B2) */}
        <div class="overview-panel activity-panel">
          <div class="panel-eyebrow">{t('activityEyebrow')}</div>
          <div class="panel-title">{t('activityTitle')}</div>
          <div class="panel-unit">{t('activityUnit')}</div>
          <div class="panel-note">{t('activityNote')}</div>

          <div class="activity-bars">
            <div class="activity-side buy">
              <div class="side-info">
                <span class="side-name">{t('buy')}</span>
                <span class="side-count">
                  / {data()?.totals.buyCount ?? 0} {t('completedUnit')}
                </span>
                <span class="side-sum">
                  {formatMoney(data()?.totals.buyPrincipal ?? '0.00')}
                </span>
              </div>
              <div class="bar-track">
                <div
                  class="bar-fill buy"
                  style={{ width: `${data()?.totals.buyShare ?? 0}%` }}
                />
              </div>
            </div>

            <div class="activity-side sell">
              <div class="side-info">
                <span class="side-name">{t('sell')}</span>
                <span class="side-count">
                  / {data()?.totals.sellCount ?? 0} {t('completedUnit')}
                </span>
                <span class="side-sum">
                  {formatMoney(data()?.totals.sellPrincipal ?? '0.00')}
                </span>
              </div>
              <div class="bar-track">
                <div
                  class="bar-fill sell"
                  style={{ width: `${data()?.totals.sellShare ?? 0}%` }}
                />
              </div>
            </div>
          </div>
        </div>

        {/* THB Flow (B3) */}
        <div class="overview-panel flow-panel">
          <div class="panel-eyebrow">{t('flowEyebrow')}</div>
          <div class="panel-title">{t('flowTitle')}</div>

          <div class="flow-values">
            <div class="flow-item flow-received">
              <span class="flow-label">{t('flowIn')}</span>
              <span class="flow-amount">
                {formatMoney(data()?.totals.received ?? '0.00')}
              </span>
            </div>
            <div class="flow-item flow-paid">
              <span class="flow-label">{t('flowOut')}</span>
              <span class="flow-amount">
                {formatMoney(data()?.totals.paid ?? '0.00')}
              </span>
            </div>
            <div class="flow-item flow-net">
              <span class="flow-net-amount">{netDisplay()}</span>
            </div>
          </div>

          <div class="panel-note">{t('flowNote1')}</div>
          <div class="panel-note">{t('flowNote2')}</div>
        </div>
      </div>

      {/* Latest five (B4) */}
      <div class="overview-panel latest-panel">
        <div class="latest-header">
          <div>
            <div class="panel-eyebrow">{t('latestEyebrow')}</div>
            <div class="panel-title">{t('latestTitle')}</div>
          </div>
          <a href="#transactions">{t('viewAll')}</a>
        </div>

        <Show
          when={(data()?.latest.length ?? 0) > 0}
          fallback={<div class="empty-state">{t('emptyOverview')}</div>}
        >
          <TransactionTable
            rows={data()!.latest}
            onOpen={handleOpenDetail}
          />
        </Show>
      </div>
    </div>
  );
}
