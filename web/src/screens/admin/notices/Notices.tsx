import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  MyNotificationItem,
  MyNotificationsData,
  MyNotificationsFilter,
} from '../../../shared/api/generated/my_notifications';
import { errorText, locale, screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './notices.css';

const t = screenT('admin.notices', table);

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function refusalText(err: unknown): string {
  if (err instanceof ApiError) {
    return errorText(err.code, err.message);
  }
  return err instanceof Error ? err.message : String(err);
}

export function Notices(_props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [items, setItems] = createSignal<MyNotificationItem[]>([]);
  const [unreadOnly, setUnreadOnly] = createSignal(false);
  const [markingIds, setMarkingIds] = createSignal<Set<string>>(new Set());
  const [markingAll, setMarkingAll] = createSignal(false);

  const load = async (filter?: MyNotificationsFilter) => {
    setLoading(true);
    setLoadError(null);
    try {
      const res = await api.view<MyNotificationsData, MyNotificationsFilter>('my_notifications', {
        filter,
      });
      const raw = res.data?.items ?? [];
      const sorted = [...raw].sort((a, b) => b.seq - a.seq);
      setItems(sorted);
    } catch (e) {
      setLoadError(refusalText(e));
    } finally {
      setLoading(false);
    }
  };

  createEffect(() => {
    const filter = unreadOnly() ? { unread: true } : undefined;
    void load(filter);
  });

  const refetch = () => {
    const filter = unreadOnly() ? { unread: true } : undefined;
    void load(filter);
  };

  const markOneRead = async (id: string) => {
    setActionError(null);
    setMarkingIds((prev) => new Set(prev).add(id));
    try {
      await api.command('POST', `/api/app/notifications/${encodeURIComponent(id)}/read`, { id }, crypto.randomUUID());
      refetch();
    } catch (e) {
      setActionError(refusalText(e));
    } finally {
      setMarkingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const markAllRead = async () => {
    setActionError(null);
    setMarkingAll(true);
    try {
      await api.command('POST', '/api/app/notifications/read-all', {}, crypto.randomUUID());
      refetch();
    } catch (e) {
      setActionError(refusalText(e));
    } finally {
      setMarkingAll(false);
    }
  };

  return (
    <div class="admin-notices">
      <div class="admin-notices-toolbar">
        <label class="admin-notices-filter">
          <input
            type="checkbox"
            class="admin-notices-filter-input"
            checked={unreadOnly()}
            onChange={(e) => setUnreadOnly(e.currentTarget.checked)}
          />
          {t('filterUnread')}
        </label>
        <button
          type="button"
          class="admin-notices-mark-all-btn"
          disabled={markingAll() || items().every((i) => i.read)}
          onClick={markAllRead}
        >
          {t('markAllRead')}
        </button>
      </div>

      <Show when={actionError()}>
        {(err) => <div class="admin-notices-action-error">{err()}</div>}
      </Show>

      <Show when={loading()}>
        <div class="admin-notices-loading">{t('loading')}</div>
      </Show>

      <Show when={!loading() && loadError()}>
        <div class="admin-notices-error">
          <span>{loadError()}</span>
          <button type="button" class="admin-notices-retry-btn" onClick={refetch}>
            {t('retry')}
          </button>
        </div>
      </Show>

      <Show when={!loading() && !loadError() && items().length === 0}>
        <div class="admin-notices-empty">{t('empty')}</div>
      </Show>

      <Show when={!loading() && !loadError() && items().length > 0}>
        <div class="admin-notices-list">
          <For each={items()}>
            {(item) => (
              <div
                class="admin-notices-item"
                classList={{ 'admin-notices-item-unread': !item.read }}
              >
                <div class="admin-notices-item-header">
                  <span class="admin-notices-title">{item.title}</span>
                  <span
                    class="admin-notices-badge"
                    classList={{
                      'admin-notices-badge-unread': !item.read,
                      'admin-notices-badge-read': item.read,
                    }}
                  >
                    {item.read ? t('readBadge') : t('unreadBadge')}
                  </span>
                  <span class="admin-notices-time">{formatTime(item.createdAt)}</span>
                </div>
                <div class="admin-notices-body">{item.body}</div>
                <Show when={!item.read}>
                  <div class="admin-notices-actions">
                    <button
                      type="button"
                      class="admin-notices-mark-btn"
                      disabled={markingIds().has(item.id)}
                      onClick={() => markOneRead(item.id)}
                    >
                      {t('markRead')}
                    </button>
                  </div>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
