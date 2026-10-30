import { createEffect, createSignal, For, onCleanup, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../api/client';
import type { MyNotificationsData } from '../../api/generated/my_notifications';
import { screenT } from '../../i18n';
import { ViewPoller } from '../../live/view-poller';
import * as table from './i18n';
import './notice-bell.css';
import { publishFreshNotices } from './notice-feed';

const t = screenT('noticeBell', table);

const DEFAULT_INTERVAL_MS = 10_000;
const TOAST_DURATION_MS = 5_000;

interface ToastItem {
  id: number;
  title: string;
  body: string;
}

let nextToastId = 0;

export function NoticeBell(props: {
  client: Pick<ApiClient, 'view'>;
  branch: string | null;
  intervalMs?: number;
}): JSX.Element {
  const [count, setCount] = createSignal(0);
  const [toasts, setToasts] = createSignal<ToastItem[]>([]);

  createEffect(() => {
    const branch = props.branch;
    const intervalMs = props.intervalMs ?? DEFAULT_INTERVAL_MS;
    if (!branch) {
      setCount(0);
      setToasts([]);
      return;
    }

    const poller = new ViewPoller<MyNotificationsData>(props.client, 'my_notifications', {
      params: { branch, filter: { unread: true } },
      intervalMs,
      maxIntervalMs: Math.max(intervalMs * 6, 60_000),
    });

    let seenFirst = false;
    let highestSeq = 0;
    const timers = new Set<ReturnType<typeof setTimeout>>();

    createEffect(() => {
      const data = poller.data();
      if (!data) return;
      const items = data.items ?? [];
      setCount(items.length);

      let maxSeq = highestSeq;
      for (const item of items) {
        if (item.seq > maxSeq) maxSeq = item.seq;
      }

      if (seenFirst) {
        const fresh = items
          .filter((item) => item.seq > highestSeq)
          .sort((a, b) => a.seq - b.seq);
        if (fresh.length > 0) {
          publishFreshNotices(
            fresh.map((item) => ({
              seq: item.seq,
              objectType: item.object?.objectType ?? null,
              objectId: item.object?.objectId ?? null,
            }))
          );
        }
        for (const item of fresh) {
          if (item.object?.objectType !== 'cash_count') continue;
          const id = ++nextToastId;
          setToasts((prev) => [...prev, { id, title: item.title, body: item.body }]);
          const timer = setTimeout(() => {
            timers.delete(timer);
            setToasts((prev) => prev.filter((entry) => entry.id !== id));
          }, TOAST_DURATION_MS);
          timers.add(timer);
        }
      }

      seenFirst = true;
      highestSeq = maxSeq;
    });

    poller.start();

    onCleanup(() => {
      poller.stop();
      for (const timer of timers) clearTimeout(timer);
      timers.clear();
    });
  });

  const label = () => (count() > 0 ? t('unreadCount', { count: count() }) : t('label'));

  return (
    <div class="notice-bell-wrap">
      <a
        class="notice-bell"
        data-testid="notice-bell"
        href="/#notices"
        aria-label={label()}
        title={label()}
      >
        <svg class="notice-bell-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M12 22a2 2 0 0 0 2-2h-4a2 2 0 0 0 2 2zm6-6V11a6 6 0 0 0-5-5.91V4a1 1 0 0 0-2 0v1.09A6 6 0 0 0 6 11v5l-2 2v1h16v-1l-2-2z"
            fill="currentColor"
          />
        </svg>
        <Show when={count() > 0}>
          <span class="notice-bell-count" data-testid="notice-bell-count">
            {count()}
          </span>
        </Show>
      </a>
      <Show when={toasts().length > 0}>
        <div class="notice-bell-toasts">
          <For each={toasts()}>
            {(toast) => (
              <div class="notice-toast" data-testid="notice-toast" role="status" aria-live="polite">
                <div class="notice-toast-title">{toast.title}</div>
                <div class="notice-toast-body">{toast.body}</div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
