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
import { Card, CardBody, CardActions } from '../../../shared/ui/Card';
import { Field, CheckField } from '../../../shared/ui/Field';
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

interface MemberFormProps {
  api: ApiClient;
  branch: string;
  customer: AdminCustomerRow;
}

function MemberForm(props: MemberFormProps): JSX.Element {
  const t = screenT('admin.customers', { th, en });
  const [phone, setPhone] = createSignal('');
  const [verified, setVerified] = createSignal(false);
  const [submitting, setSubmitting] = createSignal(false);
  const [errorSentence, setErrorSentence] = createSignal<string | null>(null);

  // The secret UUID is created ONCE when the form opens and reused on retries
  const memberSecret = crypto.randomUUID();

  async function handleSubmit(e: Event) {
    e.preventDefault();
    if (submitting()) return;
    setErrorSentence(null);
    setSubmitting(true);

    try {
      const res = await props.api.post<
        Record<string, unknown>,
        { saved: boolean; name: string; memberCode: string }
      >('/api/v2/bookings', {
        action: 'member',
        payload: {
          branchId: props.branch,
          customerId: props.customer.id,
          phone: phone().trim(),
          identityConfirmed: verified(),
          memberSecret,
        },
      });

      // On success, show member card
      openDialog(`${t('memberCardTitle')} · ${res.name}`, () => (
        <Card>
          <CardBody>
            <p>{t('memberHandOver')}</p>
            <Field label={t('memberCode')}>
              <input type="text" readOnly value={res.memberCode} />
            </Field>
            <p class="dialog-note">{t('memberCardNote')}</p>
            <CardActions>
              <button type="button" class="secondary" onClick={closeDialog}>
                {t('close')}
              </button>
            </CardActions>
          </CardBody>
        </Card>
      ));
    } catch (err: unknown) {
      setErrorSentence(wordedError(err));
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} class="member-form">
      <p class="customer-info">
        {props.customer.name} · {props.customer.id}
      </p>

      <Show when={errorSentence()}>
        {(sentence) => <div class="error-banner">{sentence()}</div>}
      </Show>

      <Field label={t('memberPhone')}>
        <input
          type="tel"
          required
          maxLength={30}
          value={phone()}
          onInput={(e) => setPhone(e.currentTarget.value)}
        />
      </Field>

      <CheckField
        label={t('memberVerified')}
        required
        checked={verified()}
        onChange={(checked) => setVerified(checked)}
      />

      <p class="reissue-note">{t('memberReissue')}</p>

      <div class="dialog-actions">
        <button type="button" class="secondary" onClick={closeDialog}>
          {t('memberBack')}
        </button>
        <button type="submit" class="primary" disabled={submitting()}>
          {t('memberSubmit')}
        </button>
      </div>
    </form>
  );
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
  const [hasMore, setHasMore] = createSignal(false);
  const [historyError, setHistoryError] = createSignal<string | null>(null);

  let lastHistorySeq = 0;
  const PAGE_LIMIT = 50;

  async function loadHistory(query: string, beforeId?: string) {
    const seq = ++lastHistorySeq;

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
      }
      setHasMore(false);
      setHistoryError(wordedError(err));
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

  function openMember() {
    openDialog(t('memberTitle'), () => (
      <MemberForm api={props.api} branch={props.branch} customer={props.customer} />
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
          <Show when={!historyError()}>
            <div class="empty">{t('historyEmpty')}</div>
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

      <button type="button" class="primary" onClick={openMember}>
        {t('memberButton')}
      </button>
    </>
  );
}

export function Customers(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.customers', { th, en });
  const api = new ApiClient();

  const [activeTab, setActiveTab] = createSignal<'customers' | 'registrations'>('customers');
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

  // Registrations tab state
  const [regSearch, setRegSearch] = createSignal('');
  const [regPage, setRegPage] = createSignal(0);
  const [regHasMore, setRegHasMore] = createSignal(false);
  const [regRefusal, setRegRefusal] = createSignal<string | null>(null);

  let lastListSeq = 0;
  let lastRegSeq = 0;

  async function fetchBranchName(branchId: string) {
    try {
      const res = await api.view<{ branches: { id: string; name: string }[] }>('admin_branches');
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

  async function readRegistrations(branchId: string, pageNum: number, queryStr: string) {
    const seq = ++lastRegSeq;
    setRegRefusal(null);

    try {
      const res = await api.post<
        Record<string, unknown>,
        { items: unknown[]; hasMore: boolean; page: number }
      >('/api/v2/bookings', {
        action: 'registrations',
        payload: {
          branchId,
          page: pageNum,
          query: queryStr.trim(),
        },
      });
      if (seq !== lastRegSeq) return;
      setRegHasMore(Boolean(res.hasMore));
    } catch (err: unknown) {
      if (seq !== lastRegSeq) return;
      setRegRefusal(wordedError(err));
    }
  }

  // Branch effect & initial load
  createEffect(() => {
    if (!props.shown()) return;
    const currentBranch = props.branch();

    fetchBranchName(currentBranch);

    // On branch change or show: reset to page 1, clear search
    setSearch('');
    setPage(1);
    setCursors([]);
    setRows([]);
    setTotalCount(null);
    setRefusalError(null);
    readCustomers(currentBranch, '');

    // Reset registrations tab
    setRegSearch('');
    setRegPage(0);
    readRegistrations(currentBranch, 0, '');
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
        readRegistrations(curBranch, regPage(), regSearch());
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

  // Registration handlers
  function doRegSearch() {
    setRegPage(0);
    readRegistrations(props.branch(), 0, regSearch());
  }

  function doRegReload() {
    readRegistrations(props.branch(), regPage(), regSearch());
  }

  function regNext() {
    const nextP = regPage() + 1;
    setRegPage(nextP);
    readRegistrations(props.branch(), nextP, regSearch());
  }

  function regPrev() {
    if (regPage() <= 0) return;
    const prevP = regPage() - 1;
    setRegPage(prevP);
    readRegistrations(props.branch(), prevP, regSearch());
  }

  function handleTabKeyDown(e: KeyboardEvent, tab: 'customers' | 'registrations') {
    let target: 'customers' | 'registrations' | null = null;
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      target = tab === 'customers' ? 'registrations' : 'customers';
    } else if (e.key === 'Home') {
      target = 'customers';
    } else if (e.key === 'End') {
      target = 'registrations';
    }
    if (target) {
      e.preventDefault();
      setActiveTab(target);
      const id = target === 'customers' ? 'tab-customers' : 'tab-registrations';
      document.getElementById(id)?.focus();
    }
  }

  const shouldShowSummary = () => {
    if (!hasLoadedOnce() || refusalError()) return false;
    if (page() !== 1 || rows().length === 0 || rows().length >= 50) return false;
    return totalCount() !== null;
  };

  return (
    <div class="customers-screen">
      <div role="tablist" aria-label={t('tabs')} class="tab-list">
        <button
          type="button"
          id="tab-customers"
          role="tab"
          aria-selected={activeTab() === 'customers' ? 'true' : 'false'}
          aria-controls="panel-customers"
          tabIndex={activeTab() === 'customers' ? 0 : -1}
          onClick={() => setActiveTab('customers')}
          onKeyDown={(e) => handleTabKeyDown(e, 'customers')}
        >
          {t('listTab')}
        </button>
        <button
          type="button"
          id="tab-registrations"
          role="tab"
          aria-selected={activeTab() === 'registrations' ? 'true' : 'false'}
          aria-controls="panel-registrations"
          tabIndex={activeTab() === 'registrations' ? 0 : -1}
          onClick={() => setActiveTab('registrations')}
          onKeyDown={(e) => handleTabKeyDown(e, 'registrations')}
        >
          {t('registrationsTab')}
        </button>
      </div>

      <div
        id="panel-customers"
        role="tabpanel"
        aria-labelledby="tab-customers"
        hidden={activeTab() !== 'customers'}
      >
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

            <Show when={!hasLoadedOnce() && loading()}>
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

              <div class="pagination">
                <button
                  type="button"
                  class="secondary"
                  disabled={page() <= 1}
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
                  disabled={!hasMore()}
                  onClick={nextPage}
                >
                  {t('next')}
                </button>
              </div>
            </Show>
          </Show>
        </Card>
      </div>

      <div
        id="panel-registrations"
        role="tabpanel"
        aria-labelledby="tab-registrations"
        hidden={activeTab() !== 'registrations'}
      >
        <Card class="registrations-card">
          <div class="reg-top-section">
            <h2>{t('regHeading')}</h2>
            <p>{t('regIntro')}</p>

            <div class="reg-search-bar">
              <input
                type="search"
                maxLength={100}
                placeholder={t('regPlaceholder')}
                value={regSearch()}
                onInput={(e) => setRegSearch(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') doRegSearch();
                }}
              />
              <button type="button" class="secondary" onClick={doRegSearch}>
                {t('regSearch')}
              </button>
              <button type="button" class="secondary" onClick={doRegReload}>
                {t('regRefresh')}
              </button>
            </div>
          </div>

          <Show
            when={!regRefusal()}
            fallback={<div class="error-banner">{regRefusal()}</div>}
          >
            <p class="reg-empty">{t('regEmpty')}</p>
          </Show>

          <div class="pagination reg-pagination">
            <button
              type="button"
              class="secondary"
              disabled={regPage() === 0}
              onClick={regPrev}
            >
              {t('prev')}
            </button>
            <span>
              {t('page')} {regPage() + 1}
            </span>
            <button
              type="button"
              class="secondary"
              disabled={!regHasMore()}
              onClick={regNext}
            >
              {t('next')}
            </button>
          </div>
        </Card>
      </div>
    </div>
  );
}
