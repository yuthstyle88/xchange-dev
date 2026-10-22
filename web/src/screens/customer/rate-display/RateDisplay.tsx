// RD-3 public customer rate display (design rate-display.md §3, §4.3), P01 v2 board (§9, PR 3558): the host lookup,
// the pollers and the no-branch / not-linked / not-found states are v1's; everything else is the boarding-pass board.
import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { readHostOrg } from '../../../shared/api/public-host';
import { screenT } from '../../../shared/i18n';
import { PublicPoller } from '../../../shared/live/public-poller';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { boardModel, type BoardRatesData } from './board/board-model';
import { boardStatus } from './board/board-status';
import { BoardShell } from './board/BoardShell';
import { FullscreenButton } from './FullscreenButton';
import { th, en, zh } from './i18n';
import { branchFromSearch } from './location';
import './rate-display.css';

export interface RateDisplayProps {
  /** Injected so tests mock at the fetch level; `main.tsx` passes the browser's `fetch`. */
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  /** `window.location.search`, e.g. `?branch=XC-B01`. */
  search: string;
  /** The rates poll interval (§9.5: 10 s, with If-None-Match). */
  ratesIntervalMs?: number;
  /** The branch-list poll interval (D4 [proposed]: 600 s). */
  branchesIntervalMs?: number;
}

function formatOpeningHours(
  hours: Record<string, Array<{ open: string; close: string }>> | null | undefined,
): string | null {
  if (!hours) return null;
  const list: string[] = [];
  for (const intervals of Object.values(hours)) {
    if (Array.isArray(intervals)) {
      for (const i of intervals) {
        if (i.open && i.close) {
          list.push(`${i.open} - ${i.close}`);
        }
      }
    }
  }
  if (list.length === 0) return null;
  return Array.from(new Set(list)).join(', ');
}

export function RateDisplay(props: RateDisplayProps): JSX.Element {
  const t = screenT('customer.rate-display', { th, en, zh });
  const branch = branchFromSearch(props.search);

  if (!branch) {
    return (
      <div class="xc-rate-display">
        <div class="xc-rate-display-message">{t('noBranch')}</div>
      </div>
    );
  }

  const [hostState, setHostState] = createSignal<'loading' | 'ready' | 'notFound' | 'error'>('loading');
  const [branchesPoller, setBranchesPoller] = createSignal<PublicPoller<PublicBranchesData> | undefined>(undefined);
  const [ratesPoller, setRatesPoller] = createSignal<PublicPoller<BoardRatesData> | undefined>(undefined);
  const [matchedBranchId, setMatchedBranchId] = createSignal<string | null | undefined>(undefined);
  let hostOrg: string | undefined;

  const MAX_HOST_BACKOFF_MS = 300_000;
  let disposed = false;
  let hostRetryTimer: ReturnType<typeof setTimeout> | undefined;
  let hostBackoffMs = Math.min(props.ratesIntervalMs ?? 60_000, MAX_HOST_BACKOFF_MS);

  const scheduleHostRetry = () => {
    if (disposed) return;
    setHostState('error');
    const delay = hostBackoffMs;
    hostBackoffMs = Math.min(hostBackoffMs * 2, MAX_HOST_BACKOFF_MS);
    if (hostRetryTimer !== undefined) {
      clearTimeout(hostRetryTimer);
    }
    hostRetryTimer = setTimeout(() => {
      hostRetryTimer = undefined;
      void loadHost();
    }, delay);
  };

  const api = new ApiClient(props.fetch);

  const loadHost = async () => {
    if (disposed) return;
    try {
      const org = await readHostOrg(api);
      if (disposed) return;
      hostOrg = org;
      setHostState('ready');

      const branchesUrl = `/api/public/orgs/${encodeURIComponent(org)}/branches`;
      const bPoller = new PublicPoller<PublicBranchesData>(
        branchesUrl,
        props.fetch,
        props.branchesIntervalMs ?? 600_000,
        600_000,
      );
      setBranchesPoller(bPoller);
      bPoller.start();
    } catch (err) {
      if (disposed) return;
      if (err instanceof ApiError && err.status === 404) {
        setHostState('notFound');
        return;
      }
      scheduleHostRetry();
    }
  };

  createEffect(() => {
    const bData = branchesPoller()?.data();
    if (!bData || matchedBranchId() !== undefined || !hostOrg) return;
    const list = bData.branches ?? [];
    const matched = list.find((b) => b.id === branch) ?? list.find((b) => b.code === branch);
    if (!matched) {
      setMatchedBranchId(null);
      return;
    }
    setMatchedBranchId(matched.id);

    const ratesUrl = `/api/public/orgs/${encodeURIComponent(hostOrg)}/branches/${encodeURIComponent(matched.code)}/rates`;
    const rPoller = new PublicPoller<BoardRatesData>(
      ratesUrl,
      props.fetch,
      props.ratesIntervalMs ?? 10_000,
      MAX_HOST_BACKOFF_MS,
    );
    setRatesPoller(rPoller);
    rPoller.start();
  });

  onMount(() => {
    void loadHost();
  });

  onCleanup(() => {
    disposed = true;
    if (hostRetryTimer !== undefined) {
      clearTimeout(hostRetryTimer);
      hostRetryTimer = undefined;
    }
    ratesPoller()?.stop();
    branchesPoller()?.stop();
  });

  const ratesData = () => ratesPoller()?.data();
  const ratesState = () => ratesPoller()?.state() ?? 'loading';
  const branchesData = () => branchesPoller()?.data();

  // A one-second tick so the 30 s hide (§9.5) happens without waiting for the next poll.
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(tick));

  const attract = new URLSearchParams(props.search).get('attract') !== '0';

  const branchEntry = () => {
    const id = matchedBranchId();
    return id ? branchesData()?.branches?.find((b) => b.id === id) : undefined;
  };
  const branchName = () => branchEntry()?.name || branch;
  const branchHours = () => formatOpeningHours(branchEntry()?.openingHours);

  const model = createMemo(() => {
    const d = ratesData();
    return d ? boardModel(d) : { headings: [], singleColumn: true, rows: [] };
  });

  const status = createMemo(() =>
    boardStatus({
      pollerState: ratesState(),
      lastOkAt: ratesPoller()?.lastOkAt(),
      now: now(),
      lineCount: ratesData()?.lines.length,
    }),
  );

  return (
    <div class="xc-rate-display">
      <Show when={hostState() === 'loading' || (hostState() === 'ready' && matchedBranchId() === undefined)}>
        <div class="xc-rate-display-loading">{t('loading')}</div>
      </Show>

      <Show when={hostState() === 'notFound'}>
        <div class="xc-rate-display-message">{t('notLinked')}</div>
      </Show>

      <Show when={hostState() === 'error'}>
        <div class="xc-rate-display-message">{t('loadFailed')}</div>
      </Show>

      <Show when={hostState() === 'ready' && matchedBranchId() !== undefined}>
        <Show
          when={matchedBranchId() !== null && ratesState() !== 'notFound'}
          fallback={<div class="xc-rate-display-message">{t('notFound')}</div>}
        >
          <FullscreenButton label={t('fullscreen')} />
          <BoardShell
            branch={branchEntry()?.code ?? ''}
            branchName={branchName()}
            model={model()}
            status={status()}
            stale={ratesData()?.stale ?? false}
            lastOkAt={ratesPoller()?.lastOkAt()}
            sourceDate={ratesData()?.sourceDate ?? null}
            snapshotId={ratesData()?.publication?.id ?? null}
            hours={branchHours()}
            bookingEnabled={branchEntry()?.bookingEnabled}
            attract={attract}
          />
        </Show>
      </Show>
    </div>
  );
}
