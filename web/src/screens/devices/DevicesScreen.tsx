import {
  createEffect,
  createSignal,
  For,
  onCleanup,
  Show,
  type JSX,
} from 'solid-js';
import {
  type ApiClient,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import { keyedWrite } from '../../shared/api/keyed-write';
import type { DeviceEventRow } from '../../shared/api/generated/device_events';
import type { DeviceJobRow } from '../../shared/api/generated/device_jobs';
import type { DeviceTransactionRow } from '../../shared/api/generated/device_transactions';
import type { DevicesOverview } from '../../shared/api/generated/devices';
import { locale, screenT, type Locale } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { ViewPoller } from '../../shared/live/view-poller';
import { sharedSession } from '../../shared/session/session';
import { AppShell } from '../../shared/ui/AppShell';
import { Card, CardGrid, CardHead } from '../../shared/ui/Card';
import { CheckField } from '../../shared/ui/Field';
import { formatMoney, MoneyText } from '../../shared/ui/MoneyText';
import { deviceNotice } from './banner';
import { eventsCsv } from './csv';
import * as table from './i18n';
import { kindLabel, statusLabel } from './labels';
import './devices.css';

if (
  typeof window !== 'undefined' &&
  (import.meta as unknown as { env?: { MODE?: string } }).env?.MODE === 'test' &&
  !document.getElementById('app')
) {
  sharedSession().dispose();
}

export interface DevicesScreenProps {
  api: Pick<ApiClient, 'view' | 'post' | 'page'> & Partial<Pick<ApiClient, 'command'>>;
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

function formatTimestamp(iso: string, currentLocale: Locale): string {
  try {
    return new Date(iso).toLocaleString(currentLocale, { timeZone: 'Asia/Bangkok' });
  } catch {
    return iso;
  }
}

export function DevicesScreen(props: DevicesScreenProps): JSX.Element {
  const t = screenT('devices', table);

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

  const effectiveBranch = () => overview()?.branchId ?? props.branch;
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

  const failedViews = new Map<string, string>();
  let messageOwner: string | null = null;
  let nonViewError = '';

  function setNonViewMessage(text: string): void {
    messageOwner = null;
    nonViewError = text;
    setMessage(text);
  }

  function markFirstReadDone(name: string): void {
    if (firstReadsDeferred && firstReadsDeferred.remaining.has(name)) {
      firstReadsDeferred.remaining.delete(name);
      if (firstReadsDeferred.remaining.size === 0) {
        firstReadsDeferred.resolve();
        firstReadsDeferred = undefined;
      }
    }
  }

  let pollGen = 0;

  const wrappedClient: Pick<ApiClient, 'view'> = {
    view: async <T,>(name: string, params?: ViewParams): Promise<ViewAnswer<T>> => {
      const myGen = pollGen;
      let res: ViewAnswer<T>;
      try {
        res = await props.api.view<T>(name, params);
      } catch (err: unknown) {
        if (myGen !== pollGen) throw err;
        const msg = wordedError(err);
        failedViews.delete(name);
        failedViews.set(name, msg);
        messageOwner = name;
        setMessage(msg);
        markFirstReadDone(name);
        throw err;
      }
      if (myGen !== pollGen) return res;
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
      if (failedViews.has(name)) {
        failedViews.delete(name);
        if (messageOwner === name) {
          const entries = [...failedViews.entries()];
          const last = entries[entries.length - 1];
          if (last) {
            messageOwner = last[0];
            setMessage(last[1]);
          } else {
            messageOwner = null;
            setMessage(nonViewError);
          }
        }
      }
      markFirstReadDone(name);
      return res;
    },
  };

  async function loadTransactions(branch: string): Promise<void> {
    try {
      const txs = await props.api.page<DeviceTransactionRow>('device_transactions', {
        branch,
        limit: 100,
      });
      nonViewError = '';
      setTransactions(txs);
      if (selectedTx() && !txs.some((item) => item.id === selectedTx())) {
        setSelectedTx('');
      }
    } catch (err: unknown) {
      setNonViewMessage(wordedError(err));
    }
  }

  async function initBranch(branch: string, isBranchSwitch = false): Promise<void> {
    pollGen += 1;
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

  let lastBranch: string | undefined;
  createEffect(() => {
    const b = props.branch;
    if (lastBranch === undefined) {
      lastBranch = b;
      void initBranch(b, false);
    } else if (lastBranch !== b) {
      lastBranch = b;
      void initBranch(b, true);
    }
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

  async function runAction(fn: () => Promise<void>): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setNonViewMessage('');
    try {
      await fn();
      await initBranch(effectiveBranch());
    } catch (err: unknown) {
      setNonViewMessage(wordedError(err));
    } finally {
      setBusy(false);
    }
  }

  let lastMappedKey: string | null = null;
  let lastResolvedId: string | null = null;
  function resolveKey(key: string): string {
    if (!props.newId) return key;
    if (lastMappedKey !== key || lastResolvedId === null) {
      lastMappedKey = key;
      lastResolvedId = props.newId();
    }
    return lastResolvedId;
  }

  const deviceCommandClient = {
    command: async <B, R>(
      method: string,
      path: string,
      body: B,
      key: string,
    ): Promise<R> => {
      const resolvedKey = resolveKey(key);
      let finalBody: unknown = body;
      const b = body as unknown as {
        action?: string;
        payload?: Record<string, unknown>;
      };
      if (b && b.action === 'queue' && b.payload) {
        if (b.payload.kind === 'PRINT_RECEIPT') {
          finalBody = {
            action: 'queue',
            payload: {
              kind: 'PRINT_RECEIPT',
              ref: b.payload.ref,
              requestId: resolvedKey,
              branchId: b.payload.branchId,
            },
          };
        } else {
          finalBody = {
            action: 'queue',
            payload: {
              ...b.payload,
              requestId: resolvedKey,
            },
          };
        }
      }
      if (typeof props.api.command === 'function') {
        return props.api.command<unknown, R>(method as 'POST', path, finalBody, resolvedKey);
      }
      return props.api.post<unknown, R>(path, finalBody);
    },
  } as unknown as ApiClient;

  const deviceWrite = keyedWrite<Record<string, unknown>, unknown>(
    deviceCommandClient,
    'POST',
    '/api/devices',
  );

  async function handleRefresh(): Promise<void> {
    await runAction(async () => {});
  }

  async function handlePrint(): Promise<void> {
    await runAction(async () => {
      await deviceWrite.send({
        action: 'queue',
        payload: {
          kind: 'PRINT_RECEIPT',
          ref: selectedTx(),
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handleDrawer(): Promise<void> {
    await runAction(async () => {
      await deviceWrite.send({
        action: 'queue',
        payload: {
          kind: 'DRAWER_REQUEST',
          ref: selectedTx(),
          reason: reason(),
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handleProcess(id: string): Promise<void> {
    await runAction(async () => {
      await deviceWrite.send({
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
      await deviceWrite.send({
        action: 'retry',
        payload: {
          id,
          branchId: effectiveBranch(),
        },
      });
    });
  }

  async function handlePrinterOnlineChange(checked: boolean): Promise<void> {
    await runAction(async () => {
      await deviceWrite.send({
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
      await deviceWrite.send({
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
    } catch (err: unknown) {
      setNonViewMessage(wordedError(err));
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
    } catch (err: unknown) {
      setNonViewMessage(wordedError(err));
    }
  }

  return (
    <AppShell
      look="devices"
      title={t('heading')}
      banner={
        <Show when={overview()} keyed>
          {(ov) => <div class="banner">{deviceNotice(ov).banner}</div>}
        </Show>
      }
      nav={
        <>
          <a class="xc-shell-brand" href="/devices">XCHANGE <span>× T.K.</span><small>DEVICES</small></a>
          <nav>
            <a href={`/?branch=${effectiveBranch()}`}>{t('adminLink')}</a>
            <a href={`/desk?branch=${effectiveBranch()}`}>{t('deskLink')}</a>
          </nav>
        </>
      }
    >
      <div>
        <section>
          <div class="heading">
            <div>
              <div class="eyebrow">{t('eyebrow')}</div>
              <h2>{t('heading')}</h2>
              <p>{t('intro')}</p>
            </div>
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

        <CardGrid columns={4}>
          <Card>
            <span class="tag">{t('printerTag')}</span>
            <CardHead title={t('printerName')} level={3} />
            <p>{t('printerText')}</p>
            <CheckField
              label={t('printerOnline')}
              checked={overview()?.printerOnline ?? false}
              onChange={handlePrinterOnlineChange}
              disabled={busy()}
            />
            <Show
              when={overview()?.printerOnline}
              fallback={<p>{t('printerOffline')}</p>}
            >
              <p>{t('printerReady')}</p>
            </Show>
          </Card>

          <Card>
            <span class="tag">{t('scanTag')}</span>
            <CardHead title={t('scanName')} level={3} />
            <p>{t('scanText')}</p>
          </Card>

          <Card>
            <span class="tag">{t('counterTag')}</span>
            <CardHead title={t('counterName')} level={3} />
            <p>{t('counterText')}</p>
            <p class="counter-notes">
              <span>{t('counterNoteCount')}</span>
              <br />
              <span>{t('counterNoteCheck')}</span>
            </p>
          </Card>

          <Card>
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
          </Card>
        </CardGrid>

        <section class="request-section">
          <div class="section-head">
            <div>
              <div class="eyebrow">{t('requestEyebrow')}</div>
              <h2>{t('requestHeading')}</h2>
            </div>
            <button type="button" onClick={handleRefresh} disabled={busy()}>
              {t('refresh')}
            </button>
          </div>

          <div class="request-grid">
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

            <button type="button" onClick={handlePrint} disabled={busy()}>
              {t('print')}
            </button>
            <button type="button" onClick={handleDrawer} disabled={busy()}>
              {t('drawer')}
            </button>
          </div>

          <div class="table-wrap">
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
                            {' '}
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
          </div>
        </section>

        <section class="events-section">
          <div class="section-head">
            <div>
              <div class="eyebrow">{t('eventsEyebrow')}</div>
              <h2>{t('eventsHeading')}</h2>
            </div>
            <div class="events-controls">
              <button
                type="button"
                onClick={handleLoadMoreEvents}
                disabled={busy()}
              >
                {t('loadMore')}
              </button>
              <button type="button" onClick={handleExportCsv} disabled={busy()}>
                {t('exportCsv')}
              </button>
            </div>
          </div>
          <p>{t('eventsNote')}</p>
          <div class="table-wrap">
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
          </div>
        </section>

        <footer>
          <p>{t('footer')}</p>
        </footer>
      </div>
    </AppShell>
  );
}
