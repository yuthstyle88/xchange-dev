import { createEffect, createSignal, type Accessor } from 'solid-js';
import { ApiClient, ApiError } from '../api/client';
import type {
  MyNotificationItem,
  MyNotificationsData,
  MyNotificationsFilter,
} from '../api/generated/my_notifications';
import { errorText } from '../i18n';

export interface NotificationsOptions {
  /**
   * When true, marking a notification as read updates state immediately
   * and rolls back if the server request fails.
   */
  optimistic?: boolean;
  initialUnreadOnly?: boolean;
}

export interface NotificationsModel {
  items: Accessor<MyNotificationItem[]>;
  loading: Accessor<boolean>;
  loadError: Accessor<string | null>;
  actionError: Accessor<string | null>;
  unreadOnly: Accessor<boolean>;
  setUnreadOnly: (value: boolean | ((prev: boolean) => boolean)) => void;
  markingIds: Accessor<Set<string>>;
  markingAll: Accessor<boolean>;
  refetch: (opts?: { silent?: boolean } | unknown) => Promise<void>;
  markOneRead: (id: string) => Promise<void>;
  markAllRead: () => Promise<void>;
}

export function refusalText(err: unknown): string {
  if (err instanceof ApiError) {
    return errorText(err.code, err.message);
  }
  return err instanceof Error ? err.message : String(err);
}

export function createNotifications(
  api: ApiClient = new ApiClient(),
  opts: NotificationsOptions = {},
): NotificationsModel {
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [items, setItems] = createSignal<MyNotificationItem[]>([]);
  const [unreadOnly, setUnreadOnly] = createSignal(opts.initialUnreadOnly ?? false);
  const [markingIds, setMarkingIds] = createSignal<Set<string>>(new Set());
  const [markingAll, setMarkingAll] = createSignal(false);

  const load = async (filter?: MyNotificationsFilter, loadOpts?: { silent?: boolean }) => {
    if (!loadOpts?.silent) {
      setLoading(true);
    }
    setLoadError(null);
    try {
      const res = await api.view<MyNotificationsData, MyNotificationsFilter>('my_notifications', {
        filter,
      });
      const raw = res.data?.items ?? [];
      const sorted = [...raw].sort((a, b) => b.seq - a.seq);
      setItems(sorted);
    } catch (e) {
      if (!loadOpts?.silent) {
        setLoadError(refusalText(e));
      }
    } finally {
      if (!loadOpts?.silent) {
        setLoading(false);
      }
    }
  };

  createEffect(() => {
    const filter = unreadOnly() ? { unread: true } : undefined;
    void load(filter);
  });

  const refetch = async (refetchOpts?: { silent?: boolean } | unknown) => {
    const isSilent = Boolean(
      typeof refetchOpts === 'object' &&
        refetchOpts !== null &&
        'silent' in refetchOpts &&
        (refetchOpts as { silent?: boolean }).silent,
    );
    const filter = unreadOnly() ? { unread: true } : undefined;
    await load(filter, { silent: isSilent });
  };

  const markOneRead = async (id: string) => {
    setActionError(null);
    setMarkingIds((prev) => new Set(prev).add(id));

    let prevItems: MyNotificationItem[] | undefined;
    if (opts.optimistic) {
      prevItems = items();
      setItems((prev) =>
        prev.map((item) => (item.id === id ? { ...item, read: true } : item)),
      );
    }

    try {
      await api.command('POST', `/api/app/notifications/${encodeURIComponent(id)}/read`, { id }, crypto.randomUUID());
      await refetch({ silent: true });
    } catch (e) {
      if (opts.optimistic && prevItems) {
        setItems(prevItems);
      }
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

    let prevItems: MyNotificationItem[] | undefined;
    if (opts.optimistic) {
      prevItems = items();
      setItems((prev) => prev.map((item) => ({ ...item, read: true })));
    }

    try {
      await api.command('POST', '/api/app/notifications/read-all', {}, crypto.randomUUID());
      await refetch({ silent: true });
    } catch (e) {
      if (opts.optimistic && prevItems) {
        setItems(prevItems);
      }
      setActionError(refusalText(e));
    } finally {
      setMarkingAll(false);
    }
  };

  return {
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
  };
}
