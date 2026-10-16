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
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { openMenu } from '../parts/hash';
import { NewTransactionButton } from '../parts/NewTransactionButton';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import { DataTable } from '../parts/DataTable';
import { TransactionDetail } from '../parts/TransactionDetail';
import { TransactionTable } from '../parts/TransactionTable';
import { canExport, downloadCsv, walkPages } from '../parts/csv';
import { LIVE_CSV_HEADERS, liveCsvRow } from '../parts/live-csv';
import { Card, CardBody, CardGrid, CardHead } from '../../../shared/ui/Card';
import { StatCard } from '../../../shared/ui/StatCard';
import { en, th } from './i18n';
import './overview.css';

export function Overview(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.overview', { th, en });
  const api = new ApiClient();

  const [data, setData] = createSignal<AdminOverviewData | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [exportError, setExportError] = createSignal<string | null>(null);
  const [exporting, setExporting] = createSignal(false);

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
      if (err instanceof ApiError && err.code === 'outOfScope') {
        setError(t('outOfScope'));
      } else {
        setError(wordedError(err));
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
      setExportError(null);
      if (branchChanged || !(period.from && period.to && period.from > period.to)) {
        setData(null);
      }
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
    if (exporting()) return;
    setExporting(true);

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
        setExportError(null);
        toast(t('noTransactionsToExport'));
        return;
      }

      const res = await api.view<{
        branches: { id: string; name: string }[];
      }>('admin_branches');
      const b = res.data?.branches?.find((x) => x.id === branch);
      const branchName = b?.name ?? branch;

      const todayBangkok = new Intl.DateTimeFormat('en-CA', {
        timeZone: 'Asia/Bangkok',
      }).format(new Date());
      const safeBranch = branchName.replace(/[^a-zA-Z0-9-]/g, '_');
      const filename = `XCHANGE-LIVE-${safeBranch}-${todayBangkok}.csv`;

      const csvRows = allRows.map((r) => liveCsvRow(branch, branchName, r));

      downloadCsv(filename, LIVE_CSV_HEADERS, csvRows);
      setExportError(null);
    } catch (err: unknown) {
      if (err instanceof ApiError && err.code === 'outOfScope') {
        setExportError(t('outOfScope'));
      } else {
        setExportError(wordedError(err));
      }
    } finally {
      setExporting(false);
    }
  }

  const netDisplay = () => {
    const net = data()?.totals.net;
    if (net === undefined) return '—';
    if (!net.startsWith('-') && net !== '0.00' && net !== '0') {
      return `+${formatMoney(net)}`;
    }
    return formatMoney(net);
  };

  return (
    <div class="overview-screen">
      <Show when={error() ?? exportError()}>
        {(err) => <div class="error-banner">{err()}</div>}
      </Show>

      {/* Top action toolbar */}
      <div class="overview-toolbar">
        <div class="toolbar-actions">
          <NewTransactionButton api={api} branch={props.branch()} />
          <Show when={canExport()}>
            <button
              type="button"
              class="btn-export"
              onClick={handleExportCsv}
              disabled={exporting()}
            >
              {t('exportCsv')}
            </button>
          </Show>
        </div>
      </div>

      {/* Status Strip (B5) */}
      <div class="status-strip">
        <div>
          <p class="eyebrow">{t('statusStripTitle')}</p>
          <strong>
            {error()
              ? t('statusError')
              : data()?.status.shiftOpen
                ? t('shiftOpen')
                : t('shiftClosed')}
          </strong>
          <span>
            {error()
              ? t('refreshHint')
              : data()?.status.shiftOpen
                ? data()?.status.openedBy
                  ? t('openedByLabel', { staff: data()!.status.openedBy! })
                  : ''
                : t('openShiftHint')}
          </span>
        </div>
        <a href="#cash">{t('viewCashLink')}</a>
        <div>
          <p class="eyebrow">{t('pendingTotalLabel')}</p>
          <strong>
            {error() || !data()
              ? '—'
              : t('pendingItemCount', {
                  count: data()!.status.pendingCount,
                })}
          </strong>
          <span>{t('pendingNote')}</span>
        </div>
        <button
          type="button"
          class="secondary"
          disabled={error() !== null}
          onClick={handleCheckPending}
        >
          {t('checkPendingBtn')}
        </button>
      </div>

      {/* Four Stat Cards (B1) */}
      <CardGrid>
        <StatCard
          label={t('totalVolume')}
          value={data()?.totals.volume ?? '—'}
          unit="THB"
          note={t('totalVolumeNote')}
        />
        <StatCard
          label={t('buyCardLabel')}
          value={data()?.totals.buyPrincipal ?? '—'}
          unit="THB"
          note={t('buyCardNote', {
            count: data()?.totals.buyCount ?? '—',
          })}
        />
        <StatCard
          label={t('sellCardLabel')}
          value={data()?.totals.sellPrincipal ?? '—'}
          unit="THB"
          note={t('sellCardNote', {
            count: data()?.totals.sellCount ?? '—',
          })}
        />
        <StatCard
          label={t('completedTransactions')}
          value={String(data()?.totals.completedCount ?? '—')}
          unit={t('completedUnit')}
          note={t('completedNote', {
            cancelled: data()?.totals.cancelledCount ?? '—',
            pending: data()?.totals.pendingCount ?? '—',
          })}
        />
      </CardGrid>

      {/* Activity and Flow row (B2 & B3) */}
      <div class="overview-grid">
        {/* Activity BUY/SELL bars (B2) */}
        <Card class="activity-panel">
          <CardHead
            eyebrow={t('activityEyebrow')}
            title={t('activityTitle')}
            aside={<span class="unit">{t('activityUnit')}</span>}
          />
          <div id="activity">
            <div class="activity-row">
              <div class="activity-caption">
                <span>
                  {t('buy')}{' '}
                  <span class="unit">
                    / {data()?.totals.buyCount ?? '—'} {t('completedUnit')}
                  </span>
                </span>
                <strong>
                  {formatMoney(data()?.totals.buyPrincipal ?? '—')}
                </strong>
              </div>
              <div class="activity-track">
                <div
                  class="activity-fill"
                  style={{ width: `${data()?.totals.buyShare ?? 0}%` }}
                />
              </div>
            </div>

            <div class="activity-row">
              <div class="activity-caption">
                <span>
                  {t('sell')}{' '}
                  <span class="unit">
                    / {data()?.totals.sellCount ?? '—'} {t('completedUnit')}
                  </span>
                </span>
                <strong>
                  {formatMoney(data()?.totals.sellPrincipal ?? '—')}
                </strong>
              </div>
              <div class="activity-track">
                <div
                  class="activity-fill sell"
                  style={{ width: `${data()?.totals.sellShare ?? 0}%` }}
                />
              </div>
            </div>

            <p class="activity-key">{t('activityNote')}</p>
          </div>
        </Card>

        {/* THB Flow (B3) */}
        <Card tone="dark">
          <p class="eyebrow">{t('flowEyebrow')}</p>
          <h2>{t('flowTitle')}</h2>
          <div class="flow-value">
            {netDisplay()} <span class="unit">THB</span>
          </div>
          <div class="flow-row">
            <span>{t('flowIn')}</span>
            <span>{formatMoney(data()?.totals.received ?? '—')}</span>
          </div>
          <div class="flow-row">
            <span>{t('flowOut')}</span>
            <span>{formatMoney(data()?.totals.paid ?? '—')}</span>
          </div>
          <p class="dark-note">
            {t('flowNote1')}
            <br />
            {t('flowNote2')}
          </p>
        </Card>
      </div>

      {/* Latest five (B4) */}
      <Card>
        <CardHead
          eyebrow={t('latestEyebrow')}
          title={t('latestTitle')}
          aside={<a href="#transactions" class="text-link">{t('viewAll')}</a>}
        />
        <CardBody flush>
          <Show
            when={(data()?.latest.length ?? 0) > 0}
            fallback={
              <DataTable headers={[]} rowCount={0} empty={t('emptyOverview')} />
            }
          >
            <div class="table-scroll">
              <TransactionTable
                rows={data()!.latest}
                onOpen={handleOpenDetail}
              />
            </div>
          </Show>
        </CardBody>
      </Card>
    </div>
  );
}
