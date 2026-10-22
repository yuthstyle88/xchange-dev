import { createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { readHostOrg } from '../../../shared/api/public-host';
import { screenT } from '../../../shared/i18n';
import { chime, soundOn, toggleSound } from '../../../shared/queue/queue-sound';
import { FullscreenButton } from '../../customer/rate-display/FullscreenButton';
import { branchFromSearch } from '../../customer/rate-display/location';
import { en, th, zh } from './i18n';

const tr = screenT('customer.queue-display', { th, en, zh });

export interface QueueDisplayProps {
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  search: string;
  intervalMs?: number;
}

function sanitizeEntries(
  items: Array<{ displayNumber?: string; status?: string; calledAt?: string }> | undefined,
): Array<{ displayNumber: string; status: string; calledAt: string }> {
  if (!Array.isArray(items)) return [];
  return items.map((item) => ({
    displayNumber: String(item.displayNumber ?? ''),
    status: String(item.status ?? ''),
    calledAt: String(item.calledAt ?? ''),
  }));
}

export function QueueDisplay(props: QueueDisplayProps): JSX.Element {
  const [called, setCalled] = createSignal<
    Array<{ displayNumber: string; status: string; calledAt: string }>
  >([]);
  const [recent, setRecent] = createSignal<
    Array<{ displayNumber: string; status: string; calledAt: string }>
  >([]);

  const api = new ApiClient(props.fetch);
  let org: string | null = null;
  let sinceVersion: number | null = null;

  const branch = () => branchFromSearch(props.search);

  const poll = async () => {
    try {
      const isFirst = !org;
      if (!org) {
        org = await readHostOrg(api);
      }
      const code = branch();
      if (!code) return;
      const base = `/api/public/orgs/${encodeURIComponent(org)}/branches/${encodeURIComponent(code)}/queue-display`;
      const url = sinceVersion !== null ? `${base}?since=${sinceVersion}` : base;
      const body = await api.publicGet<{
        version?: number;
        changed?: boolean;
        data?: {
          called?: Array<{ displayNumber?: string; status?: string; calledAt?: string }>;
          recent?: Array<{ displayNumber?: string; status?: string; calledAt?: string }>;
        };
      }>(url);
      if (typeof body.version === 'number') {
        sinceVersion = body.version;
      }
      if (body.changed && body.data) {
        const nextCalled = sanitizeEntries(body.data.called);
        const prevKey = called()
          .map((entry) => entry.displayNumber)
          .join(',');
        const nextKey = nextCalled.map((entry) => entry.displayNumber).join(',');
        setCalled(nextCalled);
        setRecent(sanitizeEntries(body.data.recent));
        if (!isFirst && nextKey && nextKey !== prevKey) {
          chime();
        }
      }
    } catch {
      // keep last good render
    }
  };

  onMount(() => {
    void poll();
    const ms = props.intervalMs ?? 5_000;
    const id = setInterval(() => {
      void poll();
    }, ms);
    onCleanup(() => clearInterval(id));
  });

  return (
    <main class="queue-display-screen" data-branch={branch() ?? ''}>
      <div class="queue-display-toolbar">
        <Show when={!soundOn()}>
          <button
            type="button"
            class="queue-display-sound-btn"
            onClick={() => void toggleSound()}
          >
            {tr('soundOn')}
          </button>
        </Show>
        <FullscreenButton label={tr('fullscreen')} />
      </div>
      <div class="queue-display-grid">
        <section class="queue-display-section">
          <h1 class="queue-display-heading">{tr('nowCalling')}</h1>
          <Show
            when={called().length > 0}
            fallback={<div class="queue-display-empty">{tr('empty')}</div>}
          >
            <div class="queue-display-list">
              <For each={called()}>
                {(entry) => (
                  <div
                    class="queue-display-called"
                    data-called={entry.displayNumber}
                    data-status={entry.status}
                    data-called-at={entry.calledAt}
                  >
                    {entry.displayNumber}
                  </div>
                )}
              </For>
            </div>
          </Show>
        </section>
        <section class="queue-display-section">
          <h2 class="queue-display-heading">{tr('recentCalled')}</h2>
          <div class="queue-display-list">
            <For each={recent()}>
              {(entry) => (
                <div
                  class="queue-display-recent"
                  data-recent={entry.displayNumber}
                  data-status={entry.status}
                  data-called-at={entry.calledAt}
                >
                  {entry.displayNumber}
                </div>
              )}
            </For>
          </div>
        </section>
      </div>
    </main>
  );
}
