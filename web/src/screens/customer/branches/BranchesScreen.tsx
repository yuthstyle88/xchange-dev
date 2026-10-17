// APP-12 customer branches screen (design customer-app.md §4.5, §9.1 C08).
import {
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  For,
  Show,
  type JSX,
} from 'solid-js';
import type { PublicBranchDto, PublicBranchesData } from '../../../shared/api/public/public_branches';
import { lang, screenT } from '../../../shared/i18n';
import { PublicPoller } from '../../../shared/live/public-poller';
import { formatOpeningHours } from '../../../shared/ui/openingHours';
import { th, en, zh } from './i18n';
import './branches.css';

export interface BranchesScreenProps {
  /** Injected so tests mock at the fetch level; defaults to window.fetch */
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
  /** Organisation code override; if omitted, fetched from /api/public/host */
  org?: string;
  /** The branch-list poll interval (D4: 600 s). */
  pollIntervalMs?: number;
}

export function BranchesScreen(props: BranchesScreenProps): JSX.Element {
  const t = screenT('customer.branches', { th, en, zh });
  const fetchFn = props.fetch ?? ((url, init) => window.fetch(url, init));

  const [hostState, setHostState] = createSignal<'loading' | 'ready' | 'notFound' | 'error'>(
    props.org ? 'ready' : 'loading',
  );
  const [resolvedOrg, setResolvedOrg] = createSignal<string | undefined>(props.org);
  const [poller, setPoller] = createSignal<PublicPoller<PublicBranchesData> | undefined>(undefined);
  const [searchQuery, setSearchQuery] = createSignal('');

  const initPoller = (org: string) => {
    const branchesUrl = `/api/public/orgs/${encodeURIComponent(org)}/branches`;
    const p = new PublicPoller<PublicBranchesData>(
      branchesUrl,
      fetchFn,
      props.pollIntervalMs ?? 600_000,
      600_000,
    );
    setPoller(p);
    p.start();
  };

  const resolveHost = async () => {
    if (props.org) {
      setResolvedOrg(props.org);
      setHostState('ready');
      initPoller(props.org);
      return;
    }
    setHostState('loading');
    try {
      const res = await fetchFn('/api/public/host');
      if (res.status === 200) {
        const json = (await res.json()) as { org?: string };
        if (json && typeof json.org === 'string' && json.org.length > 0) {
          setResolvedOrg(json.org);
          setHostState('ready');
          initPoller(json.org);
          return;
        }
        setHostState('notFound');
        return;
      }
      if (res.status === 404) {
        setHostState('notFound');
        return;
      }
      setHostState('error');
    } catch {
      setHostState('error');
    }
  };

  onMount(() => {
    void resolveHost();
  });

  onCleanup(() => {
    poller()?.stop();
  });

  const handleRetry = () => {
    if (hostState() !== 'ready') {
      void resolveHost();
    } else {
      const p = poller();
      if (p) {
        p.stop();
        p.start();
      } else {
        const org = resolvedOrg();
        if (org) initPoller(org);
      }
    }
  };

  const branchesData = () => poller()?.data();
  const pollerState = () => poller()?.state() ?? 'loading';

  const filteredBranches = createMemo<PublicBranchDto[]>(() => {
    const list = branchesData()?.branches ?? [];
    const q = searchQuery().trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (b) =>
        b.name.toLowerCase().includes(q) ||
        b.code.toLowerCase().includes(q) ||
        (b.address && b.address.toLowerCase().includes(q)),
    );
  });

  return (
    <div class="xc-branches-screen">
      <header class="xc-branches-header">
        <div class="xc-branches-title-group">
          <h1>{t('title')}</h1>
          <p class="xc-branches-subtitle">{t('subtitle')}</p>
        </div>
      </header>

      {/* Host loading or poller initial loading */}
      <Show when={hostState() === 'loading' || (hostState() === 'ready' && pollerState() === 'loading' && !branchesData())}>
        <div class="xc-branches-loading" role="status">
          {t('loading')}
        </div>
      </Show>

      {/* Host not found */}
      <Show when={hostState() === 'notFound'}>
        <div class="xc-branches-error" role="alert">
          <p>{t('notLinked')}</p>
          <button type="button" class="xc-branches-retry-btn" onClick={handleRetry}>
            {t('retry')}
          </button>
        </div>
      </Show>

      {/* Host error */}
      <Show when={hostState() === 'error'}>
        <div class="xc-branches-error" role="alert">
          <p>{t('loadFailed')}</p>
          <button type="button" class="xc-branches-retry-btn" onClick={handleRetry}>
            {t('retry')}
          </button>
        </div>
      </Show>

      {/* Host ready */}
      <Show when={hostState() === 'ready'}>
        {/* Poller error without data */}
        <Show when={pollerState() === 'error' && !branchesData()}>
          <div class="xc-branches-error" role="alert">
            <p>{t('loadFailed')}</p>
            <button type="button" class="xc-branches-retry-btn" onClick={handleRetry}>
              {t('retry')}
            </button>
          </div>
        </Show>

        {/* Poller not found */}
        <Show when={pollerState() === 'notFound'}>
          <div class="xc-branches-error" role="alert">
            <p>{t('notFound')}</p>
            <button type="button" class="xc-branches-retry-btn" onClick={handleRetry}>
              {t('retry')}
            </button>
          </div>
        </Show>

        {/* Poller has data */}
        <Show when={branchesData()}>
          {/* Empty branches */}
          <Show when={branchesData()!.branches.length === 0}>
            <div class="xc-branches-empty">{t('empty')}</div>
          </Show>

          {/* Non-empty branches */}
          <Show when={branchesData()!.branches.length > 0}>
            <div class="xc-branches-search-bar">
              <input
                type="search"
                class="xc-branches-search-input"
                placeholder={t('searchPlaceholder')}
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                aria-label={t('searchPlaceholder')}
              />
            </div>

            <Show when={filteredBranches().length === 0}>
              <div class="xc-branches-no-results">{t('noSearchResults')}</div>
            </Show>

            <Show when={filteredBranches().length > 0}>
              <div class="xc-branches-list">
                <For each={filteredBranches()}>
                  {(branch) => (
                    <article class="xc-branch-card" data-branch-code={branch.code}>
                      <div class="xc-branch-card-header">
                        <h2 class="xc-branch-name">{branch.name}</h2>
                        <span class="xc-branch-code">{branch.code}</span>
                      </div>

                      <div class="xc-branch-badges">
                        <span
                          class={branch.queueOpen ? 'xc-badge xc-badge-success' : 'xc-badge xc-badge-muted'}
                        >
                          {branch.queueOpen ? t('queueOpen') : t('queueClosed')}
                        </span>
                        <span
                          class={branch.bookingEnabled ? 'xc-badge xc-badge-info' : 'xc-badge xc-badge-muted'}
                        >
                          {branch.bookingEnabled ? t('bookingEnabled') : t('bookingDisabled')}
                        </span>
                      </div>

                      <div class="xc-branch-details">
                        <div class="xc-branch-row xc-branch-address">
                          <span class="xc-branch-label">{t('address')}</span>
                          <span class="xc-branch-value">{branch.address || t('noAddress')}</span>
                        </div>
                        <div class="xc-branch-row xc-branch-hours">
                          <span class="xc-branch-label">{t('hours')}</span>
                          <span class="xc-branch-value">
                            {formatOpeningHours(branch.openingHours, lang()) || t('noHours')}
                          </span>
                        </div>
                        <div class="xc-branch-row xc-branch-timezone">
                          <span class="xc-branch-label">{t('timezone')}</span>
                          <span class="xc-branch-value">{branch.timezone}</span>
                        </div>
                      </div>

                      <div class="xc-branch-actions">
                        <a
                          href={`/rates?branch=${encodeURIComponent(branch.code)}`}
                          class="xc-branch-rates-link"
                        >
                          {t('viewRates')}
                        </a>
                        <Show when={branch.bookingEnabled}>
                          <a
                            href={`/app/book?branch=${encodeURIComponent(branch.code)}`}
                            class="xc-branch-book-link"
                          >
                            {t('bookingStatus')}
                          </a>
                        </Show>
                      </div>
                    </article>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
