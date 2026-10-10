// V2-transactions (#464): transactions screen component (ticket §7, §9, §10; design §12.1, §12.2, §12.6, §12.8, §12.14)

import {
  createEffect,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { hashParams } from '../parts/hash';
import { NewTransactionButton } from '../parts/NewTransactionButton';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import { TransactionDetail } from '../parts/TransactionDetail';
import { TransactionTable } from '../parts/TransactionTable';
import { downloadCsv, walkPages } from '../parts/csv';
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

  const [rows, setRows] = createSignal<TransactionRow[]>([]);
  const [hasMore, setHasMore] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let heldVersion: number | undefined = undefined;
  let activeBranch = '';
  let activePeriodFrom = '';
  let activePeriodTo = '';
  let currentFetchId = 0;

  onMount(() => {
    const onHashChange = () => {
      const h = hashParams();
      if (h.has('q')) setSearch(h.get('q') ?? '');
      if (h.has('type')) setType(h.get('type') ?? '');
      if (h.has('status')) setStatus(h.get('status') ?? '');
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
  }) {
    const fetchId = ++currentFetchId;
    const filter: Record<string, string> = {};
    if (opts.period.from) filter.from = opts.period.from;
    if (opts.period.to) filter.to = opts.period.to;
    if (opts.q) filter.q = opts.q;
    if (opts.typ) filter.type = opts.typ;
    if (opts.stat) filter.status = opts.stat;

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
      setRows([]);
      setHasMore(false);
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
    const q = search().trim();
    const typ = type();
    const stat = status();

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
        });
      }
    }
  });

  async function handleLoadMore() {
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
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      } else {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
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

      const csvRows = allRows.map((r) => liveCsvRow(branch, branchName, r));

      downloadCsv(filename, LIVE_CSV_HEADERS, csvRows);
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      }
    }
  }

  return (
    <Card class="transactions-screen">
      <div class="filters">
        <label class="search-label">
          <span>{t('searchLabel')}</span>
          <input
            type="search"
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
          <button
            type="button"
            class="secondary"
            onClick={() => void handleExportCsv()}
          >
            {t('exportCsv')}
          </button>
          <NewTransactionButton api={api} branch={props.branch()} />
        </div>
      </div>

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
            onClick={() => void handleLoadMore()}
          >
            {t('loadMore')}
          </button>
        </div>
      </Show>
    </Card>
  );
}
