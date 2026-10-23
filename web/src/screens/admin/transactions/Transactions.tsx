// V2-transactions (#464): transactions screen component (ticket §7, §9, §10; design §12.1, §12.2, §12.6, §12.8, §12.14)

import {
  createEffect,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { hashParams } from '../parts/hash';
import { NewTransactionButton } from '../parts/NewTransactionButton';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import { TransactionDetail } from '../parts/TransactionDetail';
import { TransactionTable } from '../parts/TransactionTable';
import { canExport, downloadCsv, walkPages } from '../parts/csv';
import { LIVE_CSV_HEADERS, liveCsvRow } from '../parts/live-csv';
import { Card } from '../../../shared/ui/Card';
import { en, th } from './i18n';
import './transactions.css';

export function Transactions(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.transactions', { th, en });
  const api = new ApiClient();

  const initHash = hashParams();
  const [search, setSearch] = createSignal(initHash.get('q') ?? '');
  const [type, setType] = createSignal(initHash.get('type') ?? '');
  const [status, setStatus] = createSignal(initHash.get('status') ?? '');
  const [currency, setCurrency] = createSignal(initHash.get('currency') ?? '');
  const [channel, setChannel] = createSignal(initHash.get('channel') ?? '');

  const [rows, setRows] = createSignal<TransactionRow[]>([]);
  const [hasMore, setHasMore] = createSignal(false);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [exporting, setExporting] = createSignal(false);

  let heldVersion: number | undefined = undefined;
  let activeBranch = '';
  let activePeriodFrom = '';
  let activePeriodTo = '';
  let currentFetchId = 0;

  function clearCurrency() {
    setCurrency('');
    try {
      const h = hashParams();
      if (h.has('currency')) {
        h.delete('currency');
        const qs = h.toString();
        const base = (typeof location !== 'undefined' ? location.hash.split('?')[0] : '') || '#transactions';
        if (typeof location !== 'undefined') location.hash = qs ? `${base}?${qs}` : base;
      }
    } catch {
      // ignore
    }
  }

  function clearChannel() {
    setChannel('');
    try {
      const h = hashParams();
      if (h.has('channel')) {
        h.delete('channel');
        const qs = h.toString();
        const base = (typeof location !== 'undefined' ? location.hash.split('?')[0] : '') || '#transactions';
        if (typeof location !== 'undefined') location.hash = qs ? `${base}?${qs}` : base;
      }
    } catch {
      // ignore
    }
  }

  onMount(() => {
    const onHashChange = () => {
      const h = hashParams();
      if (h.has('q')) setSearch(h.get('q') ?? '');
      if (h.has('type')) setType(h.get('type') ?? '');
      if (h.has('status')) setStatus(h.get('status') ?? '');
      setCurrency(h.get('currency') ?? '');
      setChannel(h.get('channel') ?? '');
    };
    window.addEventListener('hashchange', onHashChange);
    onCleanup(() => window.removeEventListener('hashchange', onHashChange));
  });


  async function loadFirstPage(opts: {
    isRefresh: boolean;
    branch: string;
    period: { from: string; to: string };
    q: string;
    typ: string;
    stat: string;
    currency?: string;
    channel?: string;
  }) {
    const fetchId = ++currentFetchId;
    setLoadingMore(false);
    const filter: Record<string, string> = {};
    if (opts.period.from) filter.from = opts.period.from;
    if (opts.period.to) filter.to = opts.period.to;
    if (opts.q) filter.q = opts.q;
    if (opts.typ) filter.type = opts.typ;
    if (opts.stat) filter.status = opts.stat;
    if (opts.currency) filter.currency = opts.currency;
    if (opts.channel) filter.channel = opts.channel;

    const since = opts.isRefresh ? heldVersion : undefined;

    try {
      const answer = await api.view<TransactionRow[]>('admin_transactions', {
        branch: opts.branch,
        limit: 100,
        since,
        filter,
      });
      if (fetchId !== currentFetchId) return;

      setError(null);
      heldVersion = answer.version;
      if (answer.changed && answer.data) {
        setRows(answer.data);
        setHasMore(answer.data.length === 100);
      }
    } catch (err: unknown) {
      if (fetchId !== currentFetchId) return;
      heldVersion = undefined;
      setRows([]);
      setHasMore(false);
      setError(wordedError(err));
    }
  }

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    const period = props.period();
    const q = search().trim();
    const typ = type();
    const stat = status();
    const curr = currency();
    const ch = channel();

    const branchChanged = branch !== activeBranch;
    const periodChanged =
      period.from !== activePeriodFrom || period.to !== activePeriodTo;

    if (branchChanged || periodChanged) {
      activeBranch = branch;
      activePeriodFrom = period.from;
      activePeriodTo = period.to;
      setRows([]);
      setHasMore(false);
    }
    heldVersion = undefined;

    void loadFirstPage({
      isRefresh: false,
      branch,
      period,
      q,
      typ,
      stat,
      currency: curr,
      channel: ch,
    });
  });

  let lastSeenTick = refreshTick();
  createEffect(() => {
    const tick = refreshTick();
    if (tick !== lastSeenTick) {
      lastSeenTick = tick;
      if (props.shown()) {
        const branch = props.branch();
        const period = props.period();
        const q = search().trim();
        const typ = type();
        const stat = status();
        void loadFirstPage({
          isRefresh: true,
          branch,
          period,
          q,
          typ,
          stat,
          currency: currency(),
          channel: channel(),
        });
      }
    }
  });

  async function handleLoadMore() {
    if (loadingMore()) return;
    const currentRows = rows();
    if (currentRows.length === 0) return;
    const lastId = currentRows[currentRows.length - 1]!.id;

    const branch = props.branch();
    const period = props.period();
    const q = search().trim();
    const typ = type();
    const stat = status();

    const fetchId = currentFetchId;

    const filter: Record<string, string> = {};
    if (period.from) filter.from = period.from;
    if (period.to) filter.to = period.to;
    if (q) filter.q = q;
    if (typ) filter.type = typ;
    if (stat) filter.status = stat;
    if (currency()) filter.currency = currency();
    if (channel()) filter.channel = channel();

    setLoadingMore(true);
    try {
      const nextRows = await api.page<TransactionRow>('admin_transactions', {
        branch,
        limit: 100,
        before: lastId,
        filter,
      });
      if (fetchId !== currentFetchId || props.branch() !== branch) return;
      setRows((prev) => [...prev, ...nextRows]);
      setHasMore(nextRows.length === 100);
    } catch (err: unknown) {
      if (fetchId !== currentFetchId || props.branch() !== branch) return;
      setError(wordedError(err));
    } finally {
      if (fetchId === currentFetchId) {
        setLoadingMore(false);
      }
    }
  }

  function handleOpenDetail(id: string) {
    const row = rows().find((r) => r.id === id);
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

  async function handleExportCsv() {
    if (exporting()) return;
    setExporting(true);
    setError(null);
    const branch = props.branch();
    const period = props.period();
    const q = search().trim();
    const typ = type();
    const stat = status();

    const filter: Record<string, string> = {};
    if (period.from) filter.from = period.from;
    if (period.to) filter.to = period.to;
    if (q) filter.q = q;
    if (typ) filter.type = typ;
    if (stat) filter.status = stat;
    if (currency()) filter.currency = currency();
    if (channel()) filter.channel = channel();

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

      const res = await api.view<{
        branches: { id: string; name: string }[];
      }>('admin_branches');
      const b = res.data?.branches?.find((x) => x.id === branch);
      const branchName = b ? b.name : branch;

      const todayBangkok = new Intl.DateTimeFormat('en-CA', {
        timeZone: 'Asia/Bangkok',
      }).format(new Date());
      const safeBranch = branchName.replace(/[^a-zA-Z0-9-]/g, '_');
      const filename = `XCHANGE-LIVE-${safeBranch}-${todayBangkok}.csv`;

      const csvRows = allRows.map((r) => liveCsvRow(branch, branchName, r));

      downloadCsv(filename, LIVE_CSV_HEADERS, csvRows);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setExporting(false);
    }
  }

  return (
    <Card class="transactions-screen">
      <div class="filters">
        <label class="search-label">
          <span>{t('searchLabel')}</span>
          <input
            type="search"
            maxLength={100}
            placeholder={t('searchPlaceholder')}
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
        </label>
        <label>
          <span>{t('typeLabel')}</span>
          <select
            value={type()}
            onChange={(e) => setType(e.currentTarget.value)}
          >
            <option value="">{t('typeAll')}</option>
            <option value="BUY">{t('typeBuy')}</option>
            <option value="SELL">{t('typeSell')}</option>
          </select>
        </label>
        <label>
          <span>{t('statusLabel')}</span>
          <select
            value={status()}
            onChange={(e) => setStatus(e.currentTarget.value)}
          >
            <option value="">{t('statusAll')}</option>
            <option value="COMPLETED">{t('statusCompleted')}</option>
            <option value="CANCELLED">{t('statusCancelled')}</option>
            <option value="PENDING">{t('statusPending')}</option>
          </select>
        </label>
        <div class="actions-group">
          <Show when={canExport()}>
            <button
              type="button"
              class="secondary"
              disabled={exporting()}
              onClick={() => void handleExportCsv()}
            >
              {t('exportCsv')}
            </button>
          </Show>
          <NewTransactionButton api={api} branch={props.branch()} />
        </div>
      </div>

      <Show when={currency() || channel()}>
        <div style="display: flex; gap: 8px; padding: 0 24px 12px; align-items: center; flex-wrap: wrap;">
          <Show when={currency()}>
            {(c) => (
              <span data-filter="currency" style="display: inline-flex; align-items: center; gap: 6px; padding: 2px 8px; border-radius: 4px; background: var(--bg-subtle, #f3f4f6); font-size: 13px;">
                <span>{t('filterCurrency')}: {c()}</span>
                <button
                  type="button"
                  data-clear="currency"
                  aria-label={`${t('clearFilter')} ${t('filterCurrency')}`}
                  style="border: none; background: transparent; cursor: pointer; font-size: 14px; line-height: 1; padding: 0 2px;"
                  onClick={clearCurrency}
                >
                  ×
                </button>
              </span>
            )}
          </Show>
          <Show when={channel()}>
            {(ch) => (
              <span data-filter="channel" style="display: inline-flex; align-items: center; gap: 6px; padding: 2px 8px; border-radius: 4px; background: var(--bg-subtle, #f3f4f6); font-size: 13px;">
                <span>{t('filterChannel')}: {ch()}</span>
                <button
                  type="button"
                  data-clear="channel"
                  aria-label={`${t('clearFilter')} ${t('filterChannel')}`}
                  style="border: none; background: transparent; cursor: pointer; font-size: 14px; line-height: 1; padding: 0 2px;"
                  onClick={clearChannel}
                >
                  ×
                </button>
              </span>
            )}
          </Show>
        </div>
      </Show>

      <Show when={error()}>
        {(err) => (
          <div class="error" role="alert">
            {err()}
          </div>
        )}
      </Show>

      <div class="table-meta">
        {t('foundSummary', { count: rows().length })}
      </div>

      <div class="table-scroll">
        <TransactionTable rows={rows()} onOpen={handleOpenDetail} />
      </div>

      <Show when={hasMore()}>
        <div class="load-more-wrapper">
          <button
            type="button"
            class="secondary"
            disabled={loadingMore()}
            onClick={() => void handleLoadMore()}
          >
            {t('loadMore')}
          </button>
        </div>
      </Show>
    </Card>
  );
}
