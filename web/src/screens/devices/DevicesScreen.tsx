import {
  createEffect,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import {
  ApiError,
  type ApiClient,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import type { DeviceEventRow } from '../../shared/api/generated/device_events';
import type { DeviceJobRow } from '../../shared/api/generated/device_jobs';
import type { DeviceTransactionRow } from '../../shared/api/generated/device_transactions';
import type { DevicesOverview } from '../../shared/api/generated/devices';
import { errorText, locale, screenT } from '../../shared/i18n';
import { ViewPoller } from '../../shared/live/view-poller';
import { sharedSession } from '../../shared/session/session';
import { AppShell } from '../../shared/ui/AppShell';
import { formatMoney, MoneyText } from '../../shared/ui/MoneyText';
import { deviceNotice } from './banner';
import { eventsCsv } from './csv';
import * as table from './i18n';
import { kindLabel, statusLabel } from './labels';
import './devices.css';

if (typeof window !== 'undefined' && (import.meta as unknown as { env?: { MODE?: string } }).env?.MODE === 'test') {
  sharedSession().dispose();
}

export interface DevicesScreenProps {
  api: Pick<ApiClient, 'view' | 'post' | 'page'>;
  branch: string;
  newId?: () => string;
  navigate?: (url: string) => void;
  download?: (name: string, blob: Blob) => void;
}

function defaultDownload(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function formatTimestamp(iso: string, currentLocale: 'th-TH' | 'en-GB'): string {
  try {
    return new Date(iso).toLocaleString(currentLocale, { timeZone: 'Asia/Bangkok' });
  } catch {
    return iso;
  }
}

export function DevicesScreen(props: DevicesScreenProps): JSX.Element {
  const t = screenT('devices', table);

  const [selectedBranch, setSelectedBranch] = createSignal(props.branch || 'MAIN');
  const [overview, setOverview] = createSignal<DevicesOverview | undefined>(undefined);
  const [jobs, setJobs] = createSignal<DeviceJobRow[]>([]);
  const [events, setEvents] = createSignal<DeviceEventRow[]>([]);
  const [olderEvents, setOlderEvents] = createSignal<DeviceEventRow[]>([]);
  const [transactions, setTransactions] = createSignal<DeviceTransactionRow[] | undefined>(undefined);
  const [selectedTx, setSelectedTx] = createSignal<string>('');
  const [reason, setReason] = createSignal<string>('');
  const [busy, setBusy] = createSignal<boolean>(false);
  const [message, setMessage] = createSignal<string>('');

  let selectRef: HTMLSelectElement | undefined;

  createEffect(() => {
    transactions();
    const cur = selectedTx();
    if (selectRef) {
      selectRef.value = cur;
    }
  });

  const effectiveBranch = () => overview()?.branchId ?? selectedBranch();
  const displayedEvents = () => [...events(), ...olderEvents()];

  let devicesPoller: ViewPoller<DevicesOverview> | undefined;
  let jobsPoller: ViewPoller<DeviceJobRow[]> | undefined;
  let eventsPoller: ViewPoller<DeviceEventRow[]> | undefined;
  let transactionsLoaded = false;
  let visibilityListener: (() => void) | undefined;

  let firstReadsDeferred: {
    remaining: Set<string>;
    resolve: () => void;
  } | undefined;

  function waitForFirstReads(): Promise<void> {
    return new Promise((resolve) => {
      firstReadsDeferred = {
        remaining: new Set(['devices', 'device_jobs', 'device_events']),
        resolve,
      };
    });
  }

  const wrappedClient: Pick<ApiClient, 'view'> = {
    view: async <T,>(name: string, params?: ViewParams): Promise<ViewAnswer<T>> => {
      const res = await props.api.view<T>(name, params);
      if (res.changed && res.data !== undefined) {
        if (name === 'devices') {
          const ov = res.data as DevicesOverview;
          setOverview(ov);
        } else if (name === 'device_jobs') {
          setJobs(res.data as DeviceJobRow[]);
        } else if (name === 'device_events') {
          setEvents(res.data as DeviceEventRow[]);
        }
      }
      if (firstReadsDeferred && firstReadsDeferred.remaining.has(name)) {
        firstReadsDeferred.remaining.delete(name);
        if (firstReadsDeferred.remaining.size === 0) {
          firstReadsDeferred.resolve();
          firstReadsDeferred = undefined;
        }
      }
      return res;
    },
  };

  async function loadTransactions(branch: string): Promise<void> {
    try {
      const txs = await props.api.page<DeviceTransactionRow>('device_transactions', {
        branch,
        limit: 100,
      });
      setTransactions(txs);
      if (selectedTx() && !txs.some((item) => item.id === selectedTx())) {
        setSelectedTx('');
      }
    } catch {
      // ignore
    }
  }

  async function initBranch(branch: string, isBranchSwitch = false): Promise<void> {
    devicesPoller?.stop();
    jobsPoller?.stop();
    eventsPoller?.stop();
    if (visibilityListener) {
      document.removeEventListener('visibilitychange', visibilityListener);
      visibilityListener = undefined;
    }

    if (isBranchSwitch) {
      setOlderEvents([]);
      setTransactions(undefined);
      setSelectedTx('');
      transactionsLoaded = false;
    }

    devicesPoller = new ViewPoller<DevicesOverview>(wrappedClient, 'devices', {
      params: { branch },
      intervalMs: 10_000,
      maxIntervalMs: 60_000,
    });
    jobsPoller = new ViewPoller<DeviceJobRow[]>(wrappedClient, 'device_jobs', {
      params: { branch, limit: 100 },
      intervalMs: 10_000,
      maxIntervalMs: 60_000,
    });
    eventsPoller = new ViewPoller<DeviceEventRow[]>(wrappedClient, 'device_events', {
      params: { branch, limit: 100 },
      intervalMs: 10_000,
      maxIntervalMs: 60_000,
    });

    devicesPoller.start();
    jobsPoller.start();
    eventsPoller.start();

    if (typeof document !== 'undefined' && document.hidden) {
      transactionsLoaded = false;
      visibilityListener = () => {
        if (!document.hidden && !transactionsLoaded) {
          transactionsLoaded = true;
          void loadTransactions(branch);
          if (visibilityListener) {
            document.removeEventListener('visibilitychange', visibilityListener);
            visibilityListener = undefined;
          }
        }
      };
      document.addEventListener('visibilitychange', visibilityListener);
      return;
    }

    transactionsLoaded = true;
    const firstReadsPromise = waitForFirstReads();
    const txPromise = loadTransactions(branch);
    await Promise.all([firstReadsPromise, txPromise]);
  }

  onMount(() => {
    void initBranch(selectedBranch());
  });

  onCleanup(() => {
    devicesPoller?.stop();
    jobsPoller?.stop();
    eventsPoller?.stop();
    if (visibilityListener) {
      document.removeEventListener('visibilitychange', visibilityListener);
      visibilityListener = undefined;
    }
  });

  function handleBranchChange(e: Event & { currentTarget: HTMLSelectElement }): void {
    const newBranch = e.currentTarget.value;
    setSelectedBranch(newBranch);
    try {
      window.history.replaceState(null, '', `/devices?branch=${newBranch}`);
    } catch {
      // ignore
    }
    void initBranch(newBranch, true);
  }

  async function runAction(fn: () => Promise<void>): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setMessage('');
    try {
      await fn();
      await initBranch(effectiveBranch());
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setMessage(errorText(err.code, err.message));
      } else if (err instanceof Error) {
        setMessage(err.message);
      }
    } finally {
      setBusy(false);
    }
  }

  async function handleRefresh(): Promise<void> {
    await runAction(async () => {});
  }

  async function handlePrint(): Promise<void> {
    await runAction(async () => {
      const id = props.newId ? props.newId() : crypto.randomUUID();
      await props.api.post('/api/devices', {
        action: 'queue',
        payload: {
          kind: 'PRINT_RECEIPT',
          ref: selectedTx(),
          requestId: id,
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handleDrawer(): Promise<void> {
    await runAction(async () => {
      const id = props.newId ? props.newId() : crypto.randomUUID();
      await props.api.post('/api/devices', {
        action: 'queue',
        payload: {
          kind: 'DRAWER_REQUEST',
          ref: selectedTx(),
          reason: reason(),
          branchId: effectiveBranch(),
          requestId: id,
        },
      });
    });
  }

  async function handleProcess(id: string): Promise<void> {
    await runAction(async () => {
      await props.api.post('/api/devices', {
        action: 'process',
        payload: {
          id,
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handleRetry(id: string): Promise<void> {
    await runAction(async () => {
      await props.api.post('/api/devices', {
        action: 'retry',
        payload: {
          id,
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handlePrinterOnlineChange(e: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const checked = e.currentTarget.checked;
    await runAction(async () => {
      await props.api.post('/api/devices', {
        action: 'printerOnline',
        payload: {
          online: checked,
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handleDrawerObservation(state: 'OPEN' | 'CLOSED'): Promise<void> {
    await runAction(async () => {
      await props.api.post('/api/devices', {
        action: 'drawerObservation',
        payload: {
          state,
          ref: selectedTx(),
          branchId: effectiveBranch(),
        },
      });
    });
  }

  function handleExportCsv(): void {
    if (busy()) return;
    const csvContent = eventsCsv(displayedEvents());
    const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8' });
    const dl = props.download ?? defaultDownload;
    dl('XCHANGE-DEVICE-EVENTS.csv', blob);
  }

  async function handleLoadMoreTransactions(): Promise<void> {
    if (busy()) return;
    const current = transactions() ?? [];
    const last = current[current.length - 1];
    if (!last) return;
    try {
      const older = await props.api.page<DeviceTransactionRow>('device_transactions', {
        branch: effectiveBranch(),
        before: last.id,
        limit: 100,
      });
      const updated = [...current, ...older];
      setTransactions(updated);
      if (selectedTx() && !updated.some((item) => item.id === selectedTx())) {
        setSelectedTx('');
      }
    } catch {
      // ignore
    }
  }

  async function handleLoadMoreEvents(): Promise<void> {
    if (busy()) return;
    const current = displayedEvents();
    const last = current[current.length - 1];
    if (!last) return;
    try {
      const older = await props.api.page<DeviceEventRow>('device_events', {
        branch: effectiveBranch(),
        before: last.seq,
        limit: 100,
      });
      setOlderEvents((prev) => [...prev, ...older]);
    } catch {
      // ignore
    }
  }

  return (
    <AppShell
      look="devices"
      title={t('heading')}
      nav={
        <nav>
          <a href="/devices">XCHANGE × T.K. DEVICES</a>
          <a href={`/?branch=${effectiveBranch()}`}>{t('adminLink')}</a>
          <a href={`/desk?branch=${effectiveBranch()}`}>{t('deskLink')}</a>
        </nav>
      }
    >
      <div class="devices-page">
        <Show when={overview()} keyed>
          {(ov) => <div class="banner">{deviceNotice(ov).banner}</div>}
        </Show>

        <section class="overview-section">
          <div class="eyebrow">{t('eyebrow')}</div>
          <h2>{t('heading')}</h2>
          <p>{t('intro')}</p>

          <div class="branch-select">
            <label>
              {t('branch')}
              <select
                value={overview()?.branchId ?? selectedBranch()}
                onChange={handleBranchChange}
                disabled={busy()}
              >
                <For each={overview()?.branches ?? []}>
                  {(b) => <option value={b.id}>{b.name}</option>}
                </For>
              </select>
            </label>
          </div>

          <div id="message" role="status" aria-live="polite">
            {message()}
          </div>

          <div class="metrics">
            <div class="metric">
              <span>{t('metricThb')}</span>
              <Show when={overview()?.balanceThb} keyed>
                {(val) => <MoneyText value={val} currency="THB" />}
              </Show>
            </div>
            <div class="metric">
              <span>{t('metricUsd')}</span>
              <Show when={overview()?.balanceUsd} keyed>
                {(val) => <MoneyText value={val} currency="USD" />}
              </Show>
            </div>
            <div class="metric">
              <span>{t('metricCompleted')}</span>
              <span>{overview()?.completedCount ?? 0}</span>
            </div>
          </div>
        </section>

        <section class="cards-grid">
          <div class="card printer-card">
            <span class="tag">{t('printerTag')}</span>
            <h3>{t('printerName')}</h3>
            <p>{t('printerText')}</p>
            <label class="check">
              <input
                type="checkbox"
                checked={overview()?.printerOnline ?? false}
                onChange={handlePrinterOnlineChange}
                disabled={busy()}
              />
              {t('printerOnline')}
            </label>
            <Show
              when={overview()?.printerOnline}
              fallback={<p>{t('printerOffline')}</p>}
            >
              <p>{t('printerReady')}</p>
            </Show>
          </div>

          <div class="card scan-card">
            <span class="tag">{t('scanTag')}</span>
            <h3>{t('scanName')}</h3>
            <p>{t('scanText')}</p>
          </div>

          <div class="card counter-card">
            <span class="tag">{t('counterTag')}</span>
            <h3>{t('counterName')}</h3>
            <p>{t('counterText')}</p>
            <p class="counter-notes">
              <span>{t('counterNoteCount')}</span>
              <br />
              <span>{t('counterNoteCheck')}</span>
            </p>
          </div>

          <div class="card drawer-card">
            <Show when={overview()} keyed>
              {(ov) => <p>{deviceNotice(ov).drawer}</p>}
            </Show>
            <div class="drawer-actions">
              <button
                type="button"
                onClick={() => handleDrawerObservation('OPEN')}
                disabled={busy()}
              >
                {t('drawerOpen')}
              </button>
              <button
                type="button"
                onClick={() => handleDrawerObservation('CLOSED')}
                disabled={busy()}
              >
                {t('drawerClose')}
              </button>
            </div>
            <p class="note">{t('drawerNote')}</p>
          </div>
        </section>

        <section class="request-section">
          <div class="eyebrow">{t('requestEyebrow')}</div>
          <h2>{t('requestHeading')}</h2>
          <button type="button" onClick={handleRefresh} disabled={busy()}>
            {t('refresh')}
          </button>

          <div class="picker-group">
            <label>
              {t('transaction')}
              <select
                ref={selectRef}
                value={selectedTx()}
                onInput={(e) => setSelectedTx(e.currentTarget.value)}
                onChange={(e) => setSelectedTx(e.currentTarget.value)}
                disabled={busy()}
              >
                <Show
                  when={transactions() !== undefined}
                  fallback={<option value="">{t('noTransactions')}</option>}
                >
                  <option value="">{t('choose')}</option>
                  <For each={transactions()}>
                    {(item) => (
                      <option value={item.id}>
                        {item.id} · {item.type} {formatMoney(item.amount)} {item.currency}
                      </option>
                    )}
                  </For>
                </Show>
              </select>
            </label>
            <button
              type="button"
              onClick={handleLoadMoreTransactions}
              disabled={busy()}
            >
              {t('loadMore')}
            </button>
          </div>

          <div class="reason-group">
            <label>
              {t('reason')}
              <input
                type="text"
                maxLength={200}
                placeholder={t('reasonPlaceholder')}
                value={reason()}
                onInput={(e) => setReason(e.currentTarget.value)}
                disabled={busy()}
              />
            </label>
          </div>

          <div class="request-actions">
            <button type="button" onClick={handlePrint} disabled={busy()}>
              {t('print')}
            </button>
            <button type="button" onClick={handleDrawer} disabled={busy()}>
              {t('drawer')}
            </button>
          </div>
        </section>

        <section class="jobs-section">
          <table class="jobs-table">
            <thead>
              <tr>
                <th>{t('colJobTime')}</th>
                <th>{t('colJobKind')}</th>
                <th>{t('colJobActor')}</th>
                <th>{t('colJobStatus')}</th>
                <th>{t('colJobActions')}</th>
              </tr>
            </thead>
            <tbody>
              <Show
                when={jobs().length > 0}
                fallback={
                  <tr>
                    <td colspan="5" class="empty">{t('noJobs')}</td>
                  </tr>
                }
              >
                <For each={jobs()}>
                  {(j) => (
                    <tr>
                      <td data-label={t('colJobTime')}>
                        {formatTimestamp(j.created, locale())} <small>{j.id}</small>
                      </td>
                      <td data-label={t('colJobKind')}>
                        {kindLabel(j.kind)} <small>{j.ref || t('noRef')}</small>
                      </td>
                      <td data-label={t('colJobActor')}>{j.actor}</td>
                      <td data-label={t('colJobStatus')}>{statusLabel(j.status)}</td>
                      <td data-label={t('colJobActions')}>
                        <Show when={j.status === 'QUEUED'}>
                          <button
                            type="button"
                            onClick={() => handleProcess(j.id)}
                            disabled={busy()}
                          >
                            {t('process')}
                          </button>
                        </Show>
                        <Show when={j.status === 'FAILED'}>
                          <button
                            type="button"
                            onClick={() => handleRetry(j.id)}
                            disabled={busy()}
                          >
                            {t('retry')}
                          </button>
                        </Show>
                        <Show when={j.kind === 'PRINT_RECEIPT'}>
                          <a
                            href={`/devices/receipt?branch=${effectiveBranch()}&ref=${j.ref}`}
                            target="_blank"
                            rel="noopener noreferrer"
                          >
                            {t('receipt')}
                          </a>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </Show>
            </tbody>
          </table>
        </section>

        <section class="events-section">
          <div class="eyebrow">{t('eventsEyebrow')}</div>
          <h2>{t('eventsHeading')}</h2>
          <p>{t('eventsNote')}</p>
          <div class="events-controls">
            <button type="button" onClick={handleExportCsv} disabled={busy()}>
              {t('exportCsv')}
            </button>
            <button
              type="button"
              onClick={handleLoadMoreEvents}
              disabled={busy()}
            >
              {t('loadMore')}
            </button>
          </div>
          <table class="events-table">
            <thead>
              <tr>
                <th>{t('colTime')}</th>
                <th>{t('colActor')}</th>
                <th>{t('colEvent')}</th>
                <th>{t('colRef')}</th>
                <th>{t('colDetail')}</th>
              </tr>
            </thead>
            <tbody>
              <Show
                when={displayedEvents().length > 0}
                fallback={
                  <tr>
                    <td colspan="5" class="empty">{t('noEvents')}</td>
                  </tr>
                }
              >
                <For each={displayedEvents()}>
                  {(ev) => (
                    <tr>
                      <td data-label={t('colTime')}>{formatTimestamp(ev.at, locale())}</td>
                      <td data-label={t('colActor')}>{ev.actor}</td>
                      <td data-label={t('colEvent')}>{kindLabel(ev.kind)}</td>
                      <td data-label={t('colRef')}>
                        <small>{ev.ref || '—'}</small>
                      </td>
                      <td data-label={t('colDetail')}>
                        <code>{ev.detail}</code>
                      </td>
                    </tr>
                  )}
                </For>
              </Show>
            </tbody>
          </table>
        </section>

        <footer>
          <p>{t('footer')}</p>
        </footer>
      </div>
    </AppShell>
  );
}
