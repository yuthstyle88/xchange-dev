// RD-3 public customer rate display (design rate-display.md §3, §4.3).
import {
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { fill, locale, screenT } from '../../../shared/i18n';
import { LangSwitch } from '../../../shared/i18n/LangSwitch';
import { PublicPoller } from '../../../shared/live/public-poller';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { RateFreshness } from '../../../shared/ui/RateFreshness';
import { boardRows } from './board-rows';
import { FullscreenButton } from './FullscreenButton';
import { th, en, zh } from './i18n';
import { branchFromSearch } from './location';
import logoUrl from './logo.png';
import { RateBoard } from './RateBoard';
import { BookingQr } from './BookingQr';
import './rate-display.css';

export interface RateDisplayProps {
  /** Injected so tests mock at the fetch level; `main.tsx` passes the browser's `fetch`. */
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  /** `window.location.search`, e.g. `?branch=XC-B01`. */
  search: string;
  /** The rates poll interval (D4 [proposed]: 60 s). */
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
  const [pollers, setPollers] = createSignal<
    | {
        rates: PublicPoller<PublishedRatesData>;
        branches: PublicPoller<PublicBranchesData>;
      }
    | undefined
  >(undefined);

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

  const loadHost = async () => {
    if (disposed) return;
    try {
      const res = await props.fetch('/api/public/host');
      if (disposed) return;
      if (res.status === 200) {
        const json = (await res.json()) as { org?: string };
        if (disposed) return;
        if (json && typeof json.org === 'string' && json.org.length > 0) {
          const org = json.org;
          setHostState('ready');

          const ratesUrl = `/api/public/orgs/${encodeURIComponent(org)}/branches/${encodeURIComponent(branch)}/rates`;
          const branchesUrl = `/api/public/orgs/${encodeURIComponent(org)}/branches`;

          const rPoller = new PublicPoller<PublishedRatesData>(
            ratesUrl,
            props.fetch,
            props.ratesIntervalMs ?? 60_000,
            MAX_HOST_BACKOFF_MS,
          );
          const bPoller = new PublicPoller<PublicBranchesData>(
            branchesUrl,
            props.fetch,
            props.branchesIntervalMs ?? 600_000,
            600_000,
          );

          setPollers({ rates: rPoller, branches: bPoller });
          rPoller.start();
          bPoller.start();
          return;
        }
        setHostState('notFound');
        return;
      }
      if (res.status === 404) {
        setHostState('notFound');
        return;
      }
      scheduleHostRetry();
    } catch {
      scheduleHostRetry();
    }
  };

  onMount(() => {
    void loadHost();
  });

  onCleanup(() => {
    disposed = true;
    if (hostRetryTimer !== undefined) {
      clearTimeout(hostRetryTimer);
      hostRetryTimer = undefined;
    }
    const current = pollers();
    current?.rates.stop();
    current?.branches.stop();
  });

  const ratesData = () => pollers()?.rates.data();
  const ratesState = () => pollers()?.rates.state() ?? 'loading';
  const branchesData = () => pollers()?.branches.data();

  const branchTitle = () => {
    const branches = branchesData()?.branches;
    const match = branches?.find((b) => b.code === branch);
    return match?.name || branch;
  };

  const branchHours = () => {
    const branches = branchesData()?.branches;
    const match = branches?.find((b) => b.code === branch);
    return formatOpeningHours(match?.openingHours);
  };

  const rows = createMemo(() => {
    const d = ratesData();
    return d ? boardRows(d) : [];
  });

  const handleRetry = () => {
    const rPoller = pollers()?.rates;
    if (rPoller) {
      rPoller.stop();
      rPoller.start();
    }
  };

  const publishedAtFormatted = () => {
    const pubAt = ratesData()?.publication?.publishedAt;
    if (!pubAt) return '';
    return new Date(pubAt).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
  };

  return (
    <div class="xc-rate-display">
      <Show when={hostState() === 'loading'}>
        <div class="xc-rate-display-loading">{t('loading')}</div>
      </Show>

      <Show when={hostState() === 'notFound'}>
        <div class="xc-rate-display-message">{t('notLinked')}</div>
      </Show>

      <Show when={hostState() === 'error'}>
        <div class="xc-rate-display-message">{t('loadFailed')}</div>
      </Show>

      <Show when={hostState() === 'ready'}>
        <Show when={!ratesData()}>
          <Show when={ratesState() === 'loading'}>
            <div class="xc-rate-display-loading">{t('loading')}</div>
          </Show>
          <Show when={ratesState() === 'notFound'}>
            <div class="xc-rate-display-message">{t('notFound')}</div>
          </Show>
          <Show when={ratesState() === 'error'}>
            <div class="xc-rate-display-error">
              <p>{t('loadFailed')}</p>
              <button
                type="button"
                class="xc-rate-display-retry-btn"
                onClick={handleRetry}
              >
                {t('retry')}
              </button>
            </div>
          </Show>
        </Show>

        <Show when={ratesData()}>
          <header class="xc-rate-display-header">
            <div class="xc-rate-display-header-left">
              <img src={logoUrl} alt="Logo" class="xc-rate-display-logo" />
              <h1 class="xc-rate-display-title">{branchTitle()}</h1>
            </div>
            <div class="xc-rate-display-header-right">
              <LangSwitch />
              <FullscreenButton label={t('fullscreen')} />
            </div>
          </header>

          <Show when={branchHours()}>
            <div class="xc-rate-display-hours">
              <span class="xc-rate-display-hours-label">{t('hours')}: </span>
              <span class="xc-rate-display-hours-times">{branchHours()}</span>
            </div>
          </Show>

          <BookingQr branchCode={branch} bookingEnabled={branchesData()?.branches?.find((b) => b.code === branch)?.bookingEnabled} />

          <Show when={ratesData()?.publication}>
            <RateFreshness
              stale={ratesData()!.stale}
              sourceDate={ratesData()!.sourceDate}
              publishedAt={ratesData()!.publication!.publishedAt}
            />
          </Show>

          <Show when={ratesState() === 'error' && ratesData()?.publication}>
            <div class="xc-rate-display-banner" role="alert">
              {fill(t('updateFailed'), { at: publishedAtFormatted() })}
            </div>
          </Show>

          <Show
            when={rows().length > 0}
            fallback={<div class="xc-rate-display-empty">{t('empty')}</div>}
          >
            <RateBoard
              rows={rows()}
              currencyHeader={t('currency')}
              unitHeader={t('unit')}
              weBuyHeader={t('weBuy')}
              weSellHeader={t('weSell')}
              weBuyHint={t('weBuyHint')}
              weSellHint={t('weSellHint')}
            />
          </Show>
        </Show>
      </Show>
    </div>
  );
}
