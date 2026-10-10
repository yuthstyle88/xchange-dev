// V2-slips (#465): admin slips screen (ticket §7, §8, §9, §10; design §12.1, §12.3, §12.6, §12.14)

import {
  createEffect,
  createSignal,
  For,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { Card } from '../../../shared/ui/Card';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { refreshTick } from '../parts/refresh';
import { TransactionDetail } from '../parts/TransactionDetail';
import { formatBangkokTime } from '../parts/TransactionTable';
import { en, th } from './i18n';
import './slips.css';

export function Slips(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.slips', { th, en });
  const api = new ApiClient();

  const [search, setSearch] = createSignal('');
  const [status, setStatus] = createSignal('');
  const [page, setPage] = createSignal(1);
  const [rows, setRows] = createSignal<TransactionRow[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [branches, setBranches] = createSignal<Array<{ id: string; name: string }>>([]);

  // Keyset page cursors: cursorStack[p] is the `before` cursor to fetch page `p`.
  let cursorStack: (string | undefined)[] = [undefined, undefined];

  let currentFetchId = 0;

  async function loadBranches() {
    try {
      const res = await api.view<{ branches: Array<{ id: string; name: string }> }>('admin_branches');
      if (res.data?.branches) {
        setBranches(res.data.branches);
      }
    } catch {
      // ignore
    }
  }

  onMount(() => {
    void loadBranches();
  });

  const branchName = () => {
    const bId = props.branch();
    const b = branches().find((x) => x.id === bId);
    return b ? b.name : bId;
  };

  async function fetchSlips(targetPage: number) {
    const fetchId = ++currentFetchId;
    const branch = props.branch();
    const period = props.period();
    const q = search().trim();
    const stat = status();
    const before = cursorStack[targetPage];

    const filter: Record<string, string> = {};
    if (period.from) filter.from = period.from;
    if (period.to) filter.to = period.to;
    if (q) filter.q = q;
    if (stat) filter.status = stat;

    try {
      const data = await api.page<TransactionRow>('admin_slips', {
        branch,
        limit: 50,
        before,
        filter,
      });

      if (fetchId !== currentFetchId) return;

      setRows(data);
      setPage(targetPage);
      setError(null);
      setLoaded(true);
    } catch (err: unknown) {
      if (fetchId !== currentFetchId) return;

      setRows([]);
      setLoaded(true);
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      } else {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      }
    }
  }

  function resetAndFetch() {
    cursorStack = [undefined, undefined];
    setPage(1);
    void fetchSlips(1);
  }

  createEffect(() => {
    if (!props.shown()) return;

    // Track reactive inputs: branch, period, search, status
    props.branch();
    props.period();
    search();
    status();

    resetAndFetch();
  });

  let lastSeenTick = refreshTick();
  createEffect(() => {
    const tick = refreshTick();
    if (tick !== lastSeenTick) {
      lastSeenTick = tick;
      if (props.shown()) {
        void fetchSlips(page());
      }
    }
  });

  function handleNext() {
    const currentRows = rows();
    if (currentRows.length < 50) return;
    const lastRow = currentRows[currentRows.length - 1];
    const nextPage = page() + 1;
    cursorStack[nextPage] = lastRow.id;
    void fetchSlips(nextPage);
  }

  function handlePrev() {
    if (page() <= 1) return;
    const prevPage = page() - 1;
    void fetchSlips(prevPage);
  }

  function handleOpenDetail(row: TransactionRow) {
    openDialog(t('detailTitle'), () => (
      <TransactionDetail
        api={api}
        branch={props.branch()}
        row={row}
        onClose={closeDialog}
      />
    ));
  }

  function statusLabel(stat: string): string {
    switch (stat) {
      case 'COMPLETED':
        return t('completed');
      case 'PENDING':
        return t('pending');
      case 'CANCELLED':
        return t('cancelled');
      default:
        return stat;
    }
  }

  const summaryText = () => {
    if (error() || !loaded()) {
      return t('waiting');
    }
    if (page() === 1 && rows().length < 50) {
      return t('summary', { branch: branchName(), count: rows().length });
    }
    return '';
  };

  return (
    <Card class="slips-screen">
      <div class="filter-controls">
        <label class="filter-search">
          <span>{t('search')}</span>
          <input
            type="search"
            maxLength={100}
            placeholder={t('placeholder')}
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
        </label>
        <label>
          <span>{t('statusLabel')}</span>
          <select
            value={status()}
            onChange={(e) => setStatus(e.currentTarget.value)}
          >
            <option value="">{t('all')}</option>
            <option value="COMPLETED">{t('completed')}</option>
            <option value="PENDING">{t('pending')}</option>
            <option value="CANCELLED">{t('cancelled')}</option>
          </select>
        </label>
        <p role="status" class="summary-line">
          {summaryText()}
        </p>
      </div>

      <Show when={error()}>
        <div class="empty" role="alert">
          <strong>{t('notLoaded')}</strong>
          <p>{error()}</p>
        </div>
      </Show>

      <Show when={loaded() && rows().length === 0 && !error()}>
        <div class="empty">
          <strong>{t('empty')}</strong>
          <p>{t('emptyNote')}</p>
        </div>
      </Show>

      <Show when={rows().length > 0}>
        <div class="table-scroll">
          <table class="data-table">
            <thead>
              <tr>
                <th>{t('headerTime')}</th>
                <th>{t('headerId')}</th>
                <th>{t('headerCustomer')}</th>
                <th class="num">{t('headerSettlement')}</th>
                <th>{t('headerPaymentRef')}</th>
                <th>{t('headerStatus')}</th>
                <th>{t('headerSlip')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row) => (
                  <tr>
                    <td>
                      {row.date} {formatBangkokTime(row.created)}
                    </td>
                    <td>
                      <div>
                        <button
                          type="button"
                          class="link-button"
                          onClick={() => handleOpenDetail(row)}
                        >
                          {row.id}
                        </button>
                      </div>
                      <div>
                        {row.currency} · {row.type === 'BUY' ? t('typeBuy') : t('typeSell')}
                      </div>
                    </td>
                    <td>{row.customer}</td>
                    <td class="num">{formatMoney(row.settlement)}</td>
                    <td>{row.paymentRef || '—'}</td>
                    <td>{statusLabel(row.status)}</td>
                    <td class="slip-actions">
                      <a
                        class="secondary"
                        href={row.slipUrl ?? '#'}
                        target="_blank"
                        rel="noopener"
                      >
                        {t('openSlip')}
                      </a>
                      {' '}
                      <a
                        class="text-button"
                        href={`${row.slipUrl ?? ''}?download=1`}
                      >
                        {t('download')}
                      </a>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <div class="pagination">
        <button
          type="button"
          class="secondary"
          onClick={handlePrev}
          disabled={page() <= 1}
        >
          {t('prev')}
        </button>
        <span class="page-indicator">{t('page', { page: page() })}</span>
        <button
          type="button"
          class="secondary"
          onClick={handleNext}
          disabled={rows().length < 50}
        >
          {t('next')}
        </button>
      </div>
    </Card>
  );
}
