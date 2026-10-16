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
  ApiClient,
  type PageParams,
  type ViewParams,
} from '../api/client';
import { fill, locale, t } from '../i18n';
import { wordedError } from '../i18n/worded-error';
import { useQueueStore } from '../queue/queue-store';
import type { KioskQueueRow } from '../queue/wire';
import { MoneyText } from './MoneyText';
import './queue-panel.css';

export interface QueuePanelProps {
  branch: string;
  shown: boolean;
  onClaimed?: (id: string) => void;
  api?: ApiClient;
}

type TabState = 'waiting' | 'working' | 'completed';

export function QueuePanel(props: QueuePanelProps): JSX.Element {
  const store = useQueueStore(() => props.branch);
  const api = props.api ?? new ApiClient();

  const [activeTab, setActiveTab] = createSignal<TabState>('waiting');
  const [rows, setRows] = createSignal<KioskQueueRow[]>([]);
  const [page, setPage] = createSignal(1);
  const [loading, setLoading] = createSignal(false);
  const [readError, setReadError] = createSignal<(() => string) | string | null>(null);
  const [refusalMessage, setRefusalMessage] = createSignal<(() => string) | string | null>(null);
  const [claiming, setClaiming] = createSignal(false);
  const [nowTick, setNowTick] = createSignal(0);

  let cursorStack: Array<string | undefined> = [];
  let currentBefore: string | undefined = undefined;
  let lastQueueVersion: number | undefined = undefined;
  let currentSeq = 0;

  onMount(() => {
    const timer = setInterval(() => {
      setNowTick((tick) => tick + 1);
    }, 1000);
    onCleanup(() => clearInterval(timer));
  });

  const stamp = (iso: string | number) => {
    return new Date(iso).toLocaleString(locale(), {
      timeZone: 'Asia/Bangkok',
    });
  };

  const statusMessage = () => {
    const refusal = refusalMessage();
    if (refusal) return typeof refusal === 'function' ? refusal() : refusal;
    const err = readError();
    if (err) {
      const message = typeof err === 'function' ? err() : err;
      return fill(t('queue.status.loadFailed'), { message });
    }
    if (loading()) return t('queue.status.loading');

    const updated = store().lastUpdated();
    if (updated !== undefined) {
      return fill(t('queue.status.updated'), { time: stamp(updated) });
    }
    return '';
  };

  const readQueue = async (opts: { forced: boolean; isVersionMove?: boolean }) => {
    if (opts.forced) {
      setLoading(true);
    }
    const seq = ++currentSeq;
    const branchAtStart = props.branch;
    const tab = activeTab();
    const isPage1 = page() === 1;
    const before = currentBefore;
    const since = isPage1 && opts.isVersionMove ? lastQueueVersion : undefined;

    try {
      if (isPage1) {
        const params: ViewParams<{ state: string }> = {
          branch: branchAtStart,
          limit: 51,
          filter: { state: tab },
        };
        if (since !== undefined) {
          params.since = since;
        }
        const answer = await api.view<KioskQueueRow[]>('kiosk_queue', params);
        if (seq !== currentSeq || props.branch !== branchAtStart) return;

        if (answer.changed && answer.data) {
          setRows(answer.data);
        }
        lastQueueVersion = answer.version;
        setReadError(null);
      } else {
        const params: PageParams<{ state: string }> = {
          branch: branchAtStart,
          limit: 51,
          filter: { state: tab },
          before,
        };
        const answer = await api.page<KioskQueueRow>('kiosk_queue', params);
        if (seq !== currentSeq || props.branch !== branchAtStart) return;

        setRows(answer);
        setReadError(null);
      }
    } catch (err: unknown) {
      if (seq !== currentSeq || props.branch !== branchAtStart) return;

      setReadError(() => () => wordedError(err));
    } finally {
      if (seq === currentSeq) {
        setLoading(false);
      }
    }
  };

  let prevShown = false;
  let prevBranch: string | undefined = undefined;
  let prevVersion: number | undefined = undefined;

  createEffect(() => {
    const isShown = props.shown;
    const currentBranch = props.branch;
    const currentVer = store().version();

    if (currentBranch !== prevBranch) {
      prevBranch = currentBranch;
      prevVersion = currentVer;
      setRows([]);
      cursorStack = [];
      currentBefore = undefined;
      setPage(1);
      lastQueueVersion = undefined;
      setReadError(null);
      setRefusalMessage(null);
      if (isShown) {
        prevShown = true;
        void readQueue({ forced: true });
      }
      return;
    }

    if (isShown && !prevShown) {
      prevShown = true;
      prevVersion = currentVer;
      void readQueue({ forced: true });
      return;
    }

    if (!isShown && prevShown) {
      prevShown = false;
      return;
    }

    if (isShown) {
      if (
        prevVersion !== undefined &&
        currentVer !== undefined &&
        currentVer !== prevVersion
      ) {
        prevVersion = currentVer;
        void readQueue({ forced: false, isVersionMove: true });
      } else if (currentVer !== undefined) {
        prevVersion = currentVer;
      }
    }
  });

  const selectTab = (tab: TabState) => {
    setRows([]);
    setActiveTab(tab);
    setReadError(null);
    setRefusalMessage(null);
    cursorStack = [];
    currentBefore = undefined;
    setPage(1);
    lastQueueVersion = undefined;
    void readQueue({ forced: true });
  };

  const reload = () => {
    setRefusalMessage(null);
    void readQueue({ forced: true });
  };

  const nextPage = () => {
    if (loading()) return;
    const currentRows = rows();
    if (currentRows.length < 51) return;
    setRefusalMessage(null);
    cursorStack.push(currentBefore);
    currentBefore = currentRows[49]!.id;
    setPage((p) => p + 1);
    void readQueue({ forced: true });
  };

  const prevPage = () => {
    if (loading()) return;
    if (page() <= 1) return;
    setRefusalMessage(null);
    currentBefore = cursorStack.pop();
    setPage((p) => p - 1);
    void readQueue({ forced: true });
  };

  const claim = async (rowId: string) => {
    if (claiming()) return;
    setClaiming(true);
    setRefusalMessage(null);
    const branchAtStart = props.branch;

    try {
      await api.post('/api/kiosk', {
        action: 'accept',
        payload: { id: rowId, branchId: branchAtStart },
      });

      if (props.branch === branchAtStart) {
        setRows([]);
        setActiveTab('working');
        setReadError(null);
        cursorStack = [];
        currentBefore = undefined;
        setPage(1);
        lastQueueVersion = undefined;
        await readQueue({ forced: true });
        await store().refresh();
        props.onClaimed?.(rowId);
      } else {
        await store().refresh();
      }
    } catch (err: unknown) {
      setRefusalMessage(() => () => wordedError(err));
      await readQueue({ forced: false });
    } finally {
      setClaiming(false);
    }
  };

  const stateWord = (status: string) => {
    if (status === 'WAITING') return t('queue.state.waiting');
    if (status === 'COMPLETED') return t('queue.state.completed');
    return t('queue.state.working');
  };

  const sideText = (type: string) => {
    if (type === 'BUY') return t('queue.type.buy');
    if (type === 'SELL') return t('queue.type.sell');
    return type;
  };

  const formatDuration = (sec: number) => {
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = sec % 60;
    return fill(t('queue.duration.format'), {
      hours: h,
      minutes: m,
      seconds: s,
    });
  };

  const renderDuration = (row: KioskQueueRow) => {
    if (row.durationSeconds !== null) {
      return formatDuration(row.durationSeconds);
    }
    if (row.status !== 'COMPLETED' && row.startedAt) {
      nowTick();
      const started = Date.parse(row.startedAt);
      const elapsed = Math.max(0, Math.floor((store().now() - started) / 1000));
      return fill(t('queue.duration.runningFormat'), {
        duration: formatDuration(elapsed),
      });
    }
    return t('queue.row.noData');
  };

  const displayedRows = () => rows().slice(0, 50);

  return (
    <div class="xc-queue-panel">
      <nav class="xc-queue-tabs" aria-label={t('queue.tabsLabel')}>
        <button
          type="button"
          aria-pressed={activeTab() === 'waiting' ? 'true' : 'false'}
          onClick={() => selectTab('waiting')}
        >
          {t('queue.state.waiting')}
        </button>
        <button
          type="button"
          aria-pressed={activeTab() === 'working' ? 'true' : 'false'}
          onClick={() => selectTab('working')}
        >
          {t('queue.state.working')}
        </button>
        <button
          type="button"
          aria-pressed={activeTab() === 'completed' ? 'true' : 'false'}
          onClick={() => selectTab('completed')}
        >
          {t('queue.state.completed')}
        </button>
        <button type="button" onClick={reload}>
          {t('queue.tab.reload')}
        </button>
      </nav>

      <p class="queue-message" role="status" aria-live="polite">
        {statusMessage()}
      </p>

      <Show
        when={displayedRows().length > 0}
        fallback={
          <Show when={!loading() && readError() === null}>
            <p class="queue-empty">{t('queue.list.empty')}</p>
          </Show>
        }
      >
        <div class="xc-queue-list">
          <For each={displayedRows()}>
            {(row) => (
              <article class="xc-queue-card">
                <div class="queue-order-head">
                  <h3>{row.queue}</h3>
                  <span class="queue-state">{stateWord(row.status)}</span>
                </div>
                <p>
                  {sideText(row.type)} ·{' '}
                  <MoneyText value={row.foreign} currency={row.currency} /> →{' '}
                  <MoneyText value={row.thb} currency="THB" />
                </p>
                <p class="queue-refs">
                  {fill(t('queue.row.order'), {
                    orderNo: row.orderNo,
                    transactionNo: row.transactionNo ?? t('queue.row.unrecorded'),
                  })}
                </p>
                <dl class="queue-times">
                  <div>
                    <dt>{t('queue.row.staff')}</dt>
                    <dd>{row.staff ? row.staff : t('queue.row.noStaff')}</dd>
                  </div>
                  <div>
                    <dt>{t('queue.row.startedAt')}</dt>
                    <dd>{row.startedAt ? stamp(row.startedAt) : t('queue.row.noData')}</dd>
                  </div>
                  <div>
                    <dt>{t('queue.row.completedAt')}</dt>
                    <dd>
                      {row.status === 'COMPLETED'
                        ? row.completedAt
                          ? stamp(row.completedAt)
                          : t('queue.row.noHistoricalTime')
                        : '—'}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('queue.row.duration')}</dt>
                    <dd>{renderDuration(row)}</dd>
                  </div>
                </dl>

                <div class="queue-order-actions">
                  <Show when={row.canAccept}>
                    <button
                      class="queue-primary"
                      type="button"
                      disabled={readError() !== null || claiming()}
                      onClick={() => void claim(row.id)}
                    >
                      {t('queue.action.claim')}
                    </button>
                  </Show>
                  <Show when={row.canContinue}>
                    <a
                      class="queue-primary"
                      href={
                        readError() !== null
                          ? undefined
                          : `/desk?branch=${encodeURIComponent(props.branch)}&kiosk=${encodeURIComponent(row.id)}`
                      }
                    >
                      {t('queue.action.continue')}
                    </a>
                  </Show>
                  <Show
                    when={
                      !row.canContinue &&
                      (row.status === 'ACCEPTED' || row.status === 'ID_CHECKED')
                    }
                  >
                    <p class="queue-assigned">
                      {fill(t('queue.row.workingBy'), { staff: row.staff })}
                    </p>
                  </Show>
                </div>
              </article>
            )}
          </For>
        </div>
      </Show>

      <nav class="xc-queue-pager" aria-label={t('queue.pagerLabel')}>
        <button
          type="button"
          disabled={page() <= 1 || loading()}
          onClick={prevPage}
        >
          {t('queue.pager.prev')}
        </button>
        <span>{fill(t('queue.pager.page'), { page: page() })}</span>
        <button
          type="button"
          disabled={rows().length < 51 || loading()}
          onClick={nextPage}
        >
          {t('queue.pager.next')}
        </button>
      </nav>
    </div>
  );
}
