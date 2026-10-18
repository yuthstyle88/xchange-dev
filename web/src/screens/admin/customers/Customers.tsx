// V2-customers (#466): customers and member admin screen (§7, §8, §9, §10; design §12.1, §12.3, §12.6, §12.10, §12.14)

import {
  createEffect,
  createSignal,
  For,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { AdminCustomerRow, AdminCustomersFilter } from '../../../shared/api/generated/admin_customers';
import type { AdminCustomerHistoryFilter } from '../../../shared/api/generated/admin_customer_history';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { refreshTick } from '../parts/refresh';
import { TransactionDetail } from '../parts/TransactionDetail';
import { Card } from '../../../shared/ui/Card';
import { en, th } from './i18n';
import './customers.css';

function formatBangkokDateTime(iso: string): string {
  try {
    const d = new Date(iso);
    const date = d.toLocaleDateString('en-CA', { timeZone: 'Asia/Bangkok' });
    const time = d.toLocaleTimeString('en-GB', {
      timeZone: 'Asia/Bangkok',
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
    return `${date} ${time}`;
  } catch {
    return iso;
  }
}

interface CustomerHistoryProps {
  api: ApiClient;
  branch: string;
  branchName: string;
  customer: AdminCustomerRow;
}

function CustomerHistory(props: CustomerHistoryProps): JSX.Element {
  const t = screenT('admin.customers', { th, en });
  const [search, setSearch] = createSignal('');
  const [historyRows, setHistoryRows] = createSignal<TransactionRow[]>([]);
  const [historyLoading, setHistoryLoading] = createSignal(true);
  const [hasMore, setHasMore] = createSignal(false);
  const [historyError, setHistoryError] = createSignal<string | null>(null);

  let lastHistorySeq = 0;
  const PAGE_LIMIT = 50;

  async function loadHistory(query: string, beforeId?: string) {
    const seq = ++lastHistorySeq;
    setHistoryLoading(true);

    const trimmed = query.trim();
    setHistoryError(null);
    try {
      const data = await props.api.page<TransactionRow, AdminCustomerHistoryFilter>(
        'admin_customer_history',
        {
          branch: props.branch,
          limit: PAGE_LIMIT,
          before: beforeId,
          filter: {
            customer: props.customer.id,
            q: trimmed || undefined,
          },
        },
      );
      if (seq !== lastHistorySeq) return;

      if (beforeId) {
        setHistoryRows((prev) => [...prev, ...data]);
      } else {
        setHistoryRows(data);
      }
      setHasMore(data.length >= PAGE_LIMIT);
    } catch (err: unknown) {
      if (seq !== lastHistorySeq) return;
      if (!beforeId) {
        setHistoryRows([]);
        setHasMore(false);
      }
      setHistoryError(wordedError(err));
    } finally {
      if (seq === lastHistorySeq) {
        setHistoryLoading(false);
      }
    }
  }

  onMount(() => {
    loadHistory('');
  });

  function onSearchInput(val: string) {
    setSearch(val);
    loadHistory(val);
  }

  function loadOlder() {
    const rows = historyRows();
    if (rows.length === 0) return;
    const lastRow = rows[rows.length - 1];
    loadHistory(search(), lastRow?.id);
  }

  function openDetail(row: TransactionRow) {
    openDialog(t('detailTitle'), () => (
      <TransactionDetail
        api={props.api}
        branch={props.branch}
        row={row}
        onClose={closeDialog}
      />
    ));
  }

  function statusWord(st: string): string {
    if (st === 'COMPLETED') return t('completed');
    if (st === 'CANCELLED') return t('cancelled');
    if (st === 'PENDING') return t('pending');
    return st;
  }

  function typeWord(typ: string): string {
    if (typ === 'BUY') return t('typeBuy');
    if (typ === 'SELL') return t('typeSell');
    return typ;
  }

  return (
    <>
      <p class="dialog-note">
        {t('branchPrefix')} {props.branchName} · {t('allPeriods')}
      </p>

      <dl class="details">
        <div>
          <dt>{t('detailCustomerId')}</dt>
          <dd>{props.customer.id}</dd>
        </div>
        <div>
          <dt>{t('detailName')}</dt>
          <dd>{props.customer.name}</dd>
        </div>
        <div>
          <dt>{t('detailNationality')}</dt>
          <dd>{props.customer.nationality || '—'}</dd>
        </div>
        <div>
          <dt>{t('detailCreated')}</dt>
          <dd>{formatBangkokDateTime(props.customer.created)}</dd>
        </div>
        <div>
          <dt>{t('detailCompleted')}</dt>
          <dd>
            {props.customer.completed} {t('itemsUnit')}
          </dd>
        </div>
        <div>
          <dt>{t('detailVolume')}</dt>
          <dd>{formatMoney(props.customer.volume)} THB</dd>
        </div>
      </dl>

      <p class="dialog-note">{t('volumeNote')}</p>

      <div class="search-bar">
        <label>
          {t('historySearch')}
          <input
            type="search"
            maxLength={100}
            placeholder={t('historyPlaceholder')}
            value={search()}
            onInput={(e) => onSearchInput(e.currentTarget.value)}
          />
        </label>
      </div>

      <Show when={historyError()}>
        <div class="banner error">{historyError()}</div>
      </Show>

      <Show
        when={historyRows().length > 0}
        fallback={
          <Show
            when={!historyLoading()}
            fallback={<div class="waiting-text">{t('waiting')}</div>}
          >
            <Show when={!historyError()}>
              <div class="empty">{t('historyEmpty')}</div>
            </Show>
          </Show>
        }
      >
        <div class="table-scroll">
          <table class="history-table">
            <thead>
              <tr>
                <th>{t('thDateTime')}</th>
                <th>{t('thTxId')}</th>
                <th>{t('thType')}</th>
                <th class="num">{t('thAmount')}</th>
                <th class="num">{t('thValue')}</th>
                <th>{t('thStatus')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={historyRows()}>
                {(row) => (
                  <tr>
                    <td>{formatBangkokDateTime(row.created)}</td>
                    <td>
                      <button type="button" class="ref" onClick={() => openDetail(row)}>
                        {row.id}
                      </button>
                    </td>
                    <td>{typeWord(row.type)}</td>
                    <td class="num">
                      {formatMoney(row.amount)} {row.currency}
                    </td>
                    <td class="num">{row.principal}</td>
                    <td>{statusWord(row.status)}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>

        <Show when={hasMore()}>
          <div class="load-older-container">
            <button type="button" class="secondary" onClick={loadOlder}>
              {t('loadOlder')}
            </button>
          </div>
        </Show>
      </Show>

      <div class="dialog-actions">
        <button type="button" class="secondary" onClick={closeDialog}>
          {t('close')}
        </button>
      </div>
    </>
  );
}

export function Customers(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.customers', { th, en });
  const api = new ApiClient();

  const [branchName, setBranchName] = createSignal(props.branch());

  // Customers tab state
  const [search, setSearch] = createSignal('');
  const [page, setPage] = createSignal(1);
  const [cursors, setCursors] = createSignal<string[]>([]);
  const [rows, setRows] = createSignal<AdminCustomerRow[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [hasLoadedOnce, setHasLoadedOnce] = createSignal(false);
  const [hasMore, setHasMore] = createSignal(false);
  const [refusalError, setRefusalError] = createSignal<string | null>(null);
  const [totalCount, setTotalCount] = createSignal<number | null>(null);

  let lastListSeq = 0;
  let branchNameSeq = 0;

  async function fetchBranchName(branchId: string) {
    const seq = ++branchNameSeq;
    try {
      const res = await api.view<{ branches: { id: string; name: string }[] }>('admin_branches');
      if (seq !== branchNameSeq || props.branch() !== branchId) return;      
      const b = res.data?.branches?.find((x) => x.id === branchId);
      if (b) {
        setBranchName(b.name);
      }
    } catch {
      // fallback
    }
  }

  async function readCustomers(branchId: string, q: string, beforeCursor?: string) {
    const seq = ++lastListSeq;
    setLoading(true);

    const trimmed = q.trim();

    try {
      const data = await api.page<AdminCustomerRow, AdminCustomersFilter>('admin_customers', {
        branch: branchId,
        limit: 50,
        before: beforeCursor,
        filter: trimmed ? { q: trimmed } : undefined,
      });
      if (seq !== lastListSeq) return;

      setRows(data);
      setHasMore(data.length >= 50);
      setRefusalError(null);
      setHasLoadedOnce(true);

      if (!beforeCursor && !trimmed) {
        if (data.length < 50) {
          setTotalCount(data.length);
        } else {
          setTotalCount(null);
        }
      }
    } catch (err: unknown) {
      if (seq !== lastListSeq) return;
      setRows([]);
      setTotalCount(null);
      setHasMore(false);
      setHasLoadedOnce(true);
      setRefusalError(wordedError(err));
    } finally {
      if (seq === lastListSeq) {
        setLoading(false);
      }
    }
  }

  // Branch effect & initial load
  createEffect(() => {
    if (!props.shown()) return;
    const currentBranch = props.branch();

    setBranchName(currentBranch);
    fetchBranchName(currentBranch);

    // On branch change or show: reset to page 1, clear search
    setSearch('');
    setPage(1);
    setCursors([]);
    setRows([]);
    setTotalCount(null);
    setRefusalError(null);
    readCustomers(currentBranch, '');
  });

  // RefreshTick effect
  let lastSeenTick = refreshTick();
  createEffect(() => {
    const tick = refreshTick();
    if (tick !== lastSeenTick) {
      lastSeenTick = tick;
      if (props.shown()) {
        const curBranch = props.branch();
        const curCursor = page() > 1 ? cursors()[page() - 2] : undefined;
        readCustomers(curBranch, search(), curCursor);
      }
    }
  });

  function onSearchChange(val: string) {
    setSearch(val);
    setPage(1);
    setCursors([]);
    readCustomers(props.branch(), val);
  }

  function nextPage() {
    if (loading()) return;
    const currentRows = rows();
    if (currentRows.length === 0) return;
    const lastId = currentRows[currentRows.length - 1]?.id;
    if (!lastId) return;

    const nextP = page() + 1;
    setCursors((prev) => {
      const next = [...prev];
      next[page() - 1] = lastId;
      return next;
    });
    setPage(nextP);
    readCustomers(props.branch(), search(), lastId);
  }

  function prevPage() {
    if (loading()) return;
    if (page() <= 1) return;
    const prevP = page() - 1;
    setPage(prevP);
    const prevCursor = prevP > 1 ? cursors()[prevP - 2] : undefined;
    readCustomers(props.branch(), search(), prevCursor);
  }

  function openCustomerHistory(customer: AdminCustomerRow) {
    openDialog(t('historyTitle'), () => (
      <CustomerHistory
        api={api}
        branch={props.branch()}
        branchName={branchName()}
        customer={customer}
      />
    ));
  }

  const shouldShowSummary = () => {
    if (loading() || !hasLoadedOnce() || refusalError()) return false;
    if (page() !== 1 || rows().length === 0 || rows().length >= 50) return false;
    return totalCount() !== null;
  };

  return (
    <div class="customers-screen">
      <Card>
          <div class="search-section">
            <label>
              {t('search')}
              <input
                type="search"
                maxLength={100}
                placeholder={t('placeholder')}
                value={search()}
                onInput={(e) => onSearchChange(e.currentTarget.value)}
              />
            </label>

            <Show when={shouldShowSummary()}>
              <div role="status" class="summary-status">
                {t('branchPrefix')} {branchName()} · {t('customersCount')}{' '}
                {search().trim() ? totalCount() : rows().length} {t('personUnit')} · {t('foundCount')}{' '}
                {rows().length} {t('personUnit')} · {t('allPeriods')}
              </div>
            </Show>

            <Show when={loading() && rows().length === 0}>
              <div class="waiting-text">{t('waiting')}</div>
            </Show>
          </div>

          <Show when={refusalError()}>
            {(err) => (
              <div class="empty">
                <strong>{t('notLoaded')}</strong>
                <p class="waiting-text">{t('waiting')}</p>
                <p class="refusal-message">{err()}</p>
              </div>
            )}
          </Show>

          <Show when={hasLoadedOnce() && !refusalError()}>
            <Show
              when={rows().length > 0}
              fallback={
                <Show when={!loading()}>
                  <Show
                    when={search().trim().length > 0}
                    fallback={
                      <div class="empty">
                        <strong>{t('emptyBranch')}</strong>
                        <p>{t('emptyBranchNote')}</p>
                      </div>
                    }
                  >
                    <div class="empty">
                      <strong>{t('noMatch')}</strong>
                      <p>{t('noMatchNote')}</p>
                    </div>
                  </Show>
                </Show>
              }
            >
              <div class="table-scroll">
                <table class="customers-table">
                  <thead>
                    <tr>
                      <th>{t('thCustomerId')}</th>
                      <th>{t('thName')}</th>
                      <th>{t('thNationality')}</th>
                      <th class="num">{t('thCompleted')}</th>
                      <th class="num">{t('thTotalVolume')}</th>
                      <th>{t('thLastActivity')}</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={rows()}>
                      {(row) => (
                        <tr>
                          <td>
                            <button
                              type="button"
                              class="ref"
                              onClick={() => openCustomerHistory(row)}
                            >
                              {row.id}
                            </button>
                          </td>
                          <td>
                            {row.name}
                            <Show when={row.member}>
                              <br />
                              <span class="pill">{t('member')}</span>
                            </Show>
                          </td>
                          <td>{row.nationality || '—'}</td>
                          <td class="num">{row.completed}</td>
                          <td class="num">{row.volume === '0.00' ? '0.00' : formatMoney(row.volume)}</td>
                          <td>
                            {row.lastAt ? formatBangkokDateTime(row.lastAt) : t('noActivity')}
                          </td>
                          <td>
                            <button
                              type="button"
                              class="secondary"
                              onClick={() => openCustomerHistory(row)}
                            >
                              {t('showHistory')}
                            </button>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Show>

          <Show when={hasLoadedOnce() && (rows().length > 0 || page() > 1)}>
            <div class="pagination">
              <button
                type="button"
                class="secondary"
                disabled={loading() || page() <= 1}
                onClick={prevPage}
              >
                {t('prev')}
              </button>
              <span>
                {t('page')} {page()}
              </span>
              <button
                type="button"
                class="secondary"
                disabled={loading() || !hasMore()}
                onClick={nextPage}
              >
                {t('next')}
              </button>
            </div>
          </Show>
        </Card>
    </div>
  );
}
