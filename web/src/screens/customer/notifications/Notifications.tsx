import { For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { locale, screenT } from '../../../shared/i18n';
import { createNotifications } from '../../../shared/notifications';
import type { MyNotificationObject } from '../../../shared/api/generated/my_notifications';
import * as table from './i18n';
import './Notifications.css';

const t = screenT('customer.notifications', table);

export function objectHref(o: MyNotificationObject | undefined): string | undefined {
  if (!o) return undefined;
  if (o.objectType === 'rate_alert_event') return '/app/rate-alerts';
  if (o.objectType === 'rate_publication') return '/rates';
  return undefined;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

export interface NotificationsScreenProps {
  api?: ApiClient;
}

export function NotificationsScreen(props: NotificationsScreenProps): JSX.Element {
  const api = props.api ?? new ApiClient();
  const {
    items,
    loading,
    loadError,
    actionError,
    unreadOnly,
    setUnreadOnly,
    markingIds,
    markingAll,
    refetch,
    markOneRead,
    markAllRead,
  } = createNotifications(api, { optimistic: true });

  return (
    <div class="xc-notifications-screen">
      <header class="xc-notifications-header">
        <div class="xc-notifications-title-group">
          <h1>{t('title')}</h1>
          <p class="xc-notifications-subtitle">{t('subtitle')}</p>
        </div>
      </header>

      <div class="xc-notifications-toolbar">
        <label class="xc-notifications-filter">
          <input
            type="checkbox"
            class="xc-notifications-filter-input"
            checked={unreadOnly()}
            onChange={(e) => setUnreadOnly(e.currentTarget.checked)}
          />
          <span>{t('filterUnread')}</span>
        </label>
        <button
          type="button"
          class="xc-notifications-mark-all-btn"
          disabled={markingAll() || items().every((i) => i.read)}
          onClick={markAllRead}
        >
          {t('markAllRead')}
        </button>
      </div>

      <Show when={actionError()}>
        {(err) => <div class="xc-notifications-action-error">{err()}</div>}
      </Show>

      <Show when={loading()}>
        <div class="xc-notifications-loading">{t('loading')}</div>
      </Show>

      <Show when={!loading() && loadError()}>
        <div class="xc-notifications-error">
          <span class="xc-notifications-error-message">{loadError()}</span>
          <button type="button" class="xc-notifications-retry-btn" onClick={refetch}>
            {t('retry')}
          </button>
        </div>
      </Show>

      <Show when={!loading() && !loadError() && items().length === 0}>
        <div class="xc-notifications-empty">{t('empty')}</div>
      </Show>

      <Show when={!loading() && !loadError() && items().length > 0}>
        <div class="xc-notifications-list">
          <For each={items()}>
            {(item) => (
              <div
                class="xc-notifications-item"
                classList={{ 'xc-notifications-item-unread': !item.read }}
              >
                <div class="xc-notifications-item-header">
                  <span class="xc-notifications-item-title">{item.title}</span>
                  <span
                    class="xc-notifications-badge"
                    classList={{
                      'xc-notifications-badge-unread': !item.read,
                      'xc-notifications-badge-read': item.read,
                    }}
                  >
                    {item.read ? t('readBadge') : t('unreadBadge')}
                  </span>
                  <span class="xc-notifications-time">{formatTime(item.createdAt)}</span>
                </div>
                <div class="xc-notifications-body">{item.body}</div>
                <Show when={objectHref(item.object) || !item.read}>
                  <div class="xc-notifications-actions">
                    <Show when={objectHref(item.object)}>
                      <a class="xc-notifications-open" href={objectHref(item.object)}>
                        {t('open')}
                      </a>
                    </Show>
                    <Show when={!item.read}>
                      <button
                        type="button"
                        class="xc-notifications-mark-btn"
                        disabled={markingIds().has(item.id)}
                        onClick={() => markOneRead(item.id)}
                      >
                        {t('markRead')}
                      </button>
                    </Show>
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
