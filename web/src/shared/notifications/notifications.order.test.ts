import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../api/client';
import { errorText } from '../i18n';
import { makeMyNotificationItem } from '../testing/fixtures';
import { createNotifications } from './notifications';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T = Response>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const UNREAD_ITEM = makeMyNotificationItem({
  id: '1',
  seq: 10,
  locale: 'th',
  title: 'Unread Notice',
  body: 'b1',
  read: false,
  createdAt: '2026-10-13T01:00:00Z',
});

const READ_ITEM = makeMyNotificationItem({
  id: '2',
  seq: 20,
  locale: 'th',
  title: 'Read Notice',
  body: 'b2',
  read: true,
  createdAt: '2026-10-13T02:00:00Z',
});

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
  }
}

describe('createNotifications request ordering (#2311)', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('row 1: Toggle on then off; the unread answer arrives last -> Full list shown', async () => {
    let callCount = 0;
    const unreadReq = deferred<Response>();
    const fullReq = deferred<Response>();

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname === '/api/v3/views/my_notifications') {
        callCount += 1;
        if (callCount === 1) {
          return jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM, READ_ITEM] } });
        }
        if (url.searchParams.get('unread') === 'true') {
          return unreadReq.promise;
        }
        return fullReq.promise;
      }
      return jsonResponse({ error: 'not found' }, 404);
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['2', '1']);

      // Toggle unreadOnly on (starts unreadReq), then off (starts fullReq)
      model.setUnreadOnly(true);
      await flush();
      model.setUnreadOnly(false);
      await flush();

      // The second read (full list) answers first
      fullReq.resolve(
        jsonResponse({ version: 2, changed: true, data: { items: [UNREAD_ITEM, READ_ITEM] } }),
      );
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['2', '1']);

      // The first read (unread only) answers last -> must be ignored
      unreadReq.resolve(
        jsonResponse({ version: 2, changed: true, data: { items: [UNREAD_ITEM] } }),
      );
      await flush();

      expect(model.unreadOnly()).toBe(false);
      expect(model.items().map((i) => i.id)).toEqual(['2', '1']);
      dispose();
    });
  });

  it('row 2: First read answers, second still out -> loading() stays true', async () => {
    const firstReq = deferred<Response>();
    const secondReq = deferred<Response>();
    let callCount = 0;

    vi.stubGlobal('fetch', (input: RequestInfo | URL) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname === '/api/v3/views/my_notifications') {
        callCount += 1;
        if (callCount === 1) return firstReq.promise;
        return secondReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flush();
      expect(model.loading()).toBe(true);

      // Toggle unreadOnly while first read is still out -> starts second read
      model.setUnreadOnly(true);
      await flush();
      expect(callCount).toBe(2);

      // First read answers while second read is still out -> loading() must stay true
      firstReq.resolve(
        jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM, READ_ITEM] } }),
      );
      await flush();
      expect(model.loading()).toBe(true);

      // Second read answers -> loading() becomes false
      secondReq.resolve(
        jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM] } }),
      );
      await flush();
      expect(model.loading()).toBe(false);
      dispose();
    });
  });

  it("row 3: A silent refetch started before a toggle answers last -> The new filter's list stays", async () => {
    let viewCall = 0;
    const silentRefetchReq = deferred<Response>();
    const toggleUnreadReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM, READ_ITEM] } }),
          );
        }
        if (viewCall === 2) {
          // Silent refetch after markOneRead (unfiltered)
          return silentRefetchReq.promise;
        }
        // Toggle unreadOnly -> ?unread=true
        return toggleUnreadReq.promise;
      }

      if (url.pathname === '/api/app/notifications/1/read' && method === 'POST') {
        return Promise.resolve(jsonResponse({ readAt: '2026-10-13T12:00:00Z' }));
      }

      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['2', '1']);

      // Start markOneRead -> POST succeeds, then starts silent refetch (viewCall #2)
      const markPromise = model.markOneRead('1');
      await flush();
      expect(viewCall).toBe(2);

      // While silent refetch is still out, user toggles unreadOnly(true) (viewCall #3)
      model.setUnreadOnly(true);
      await flush();
      expect(viewCall).toBe(3);

      // Toggle read answers first with only unread items
      toggleUnreadReq.resolve(
        jsonResponse({ version: 2, changed: true, data: { items: [UNREAD_ITEM] } }),
      );
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['1']);

      // Silent refetch started before the toggle answers last with full list -> must be ignored
      silentRefetchReq.resolve(
        jsonResponse({
          version: 2,
          changed: true,
          data: { items: [{ ...UNREAD_ITEM, read: true }, READ_ITEM] },
        }),
      );
      await markPromise;
      await flush();

      expect(model.items().map((i) => i.id)).toEqual(['1']);
      dispose();
    });
  });

  it('row 4: Toggle unread -> Unread filter sent and list filtered', async () => {
    const initialFullReq = deferred<Response>();
    const unreadReq = deferred<Response>();
    const seenSearches: string[] = [];

    vi.stubGlobal('fetch', (input: RequestInfo | URL) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname === '/api/v3/views/my_notifications') {
        seenSearches.push(url.search);
        if (url.searchParams.get('unread') === 'true') {
          return unreadReq.promise;
        }
        return initialFullReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flush();

      // Toggle unreadOnly(true) -> sends unread=true filter
      model.setUnreadOnly(true);
      await flush();
      expect(seenSearches).toEqual(['', '?unread=true']);

      unreadReq.resolve(
        jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM] } }),
      );
      await flush();
      expect(model.items()).toHaveLength(1);
      expect(model.items()[0]?.id).toBe('1');

      // Earlier unfiltered read fails/completes late -> filtered list and null loadError are preserved
      initialFullReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'Stale error' } }, 500),
      );
      await flush();

      expect(model.loadError()).toBeNull();
      expect(model.items()).toHaveLength(1);
      expect(model.items()[0]?.id).toBe('1');
      dispose();
    });
  });

  it('row 5: Optimistic mark fails -> Rolled back with a worded error', async () => {
    let viewCall = 0;
    const staleRefetchReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM] } }),
          );
        }
        if (viewCall === 2) {
          return staleRefetchReq.promise;
        }
        return Promise.resolve(
          jsonResponse({ version: 2, changed: true, data: { items: [UNREAD_ITEM] } }),
        );
      }

      if (url.pathname === '/api/app/notifications/1/read' && method === 'POST') {
        return Promise.resolve(
          jsonResponse({ error: { code: 'roleNotAllowed', message: 'RoleNotAllowed' } }, 403),
        );
      }

      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();
      expect(model.items()[0]?.read).toBe(false);

      // Start an earlier refetch (#2) that stays in flight, superseded by a newer refetch (#3)
      const stalePromise = model.refetch();
      await flush();
      await model.refetch();
      await flush();

      // Optimistic markOneRead immediately sets read = true, then rolls back on 403 with wordedError
      const markPromise = model.markOneRead('1');
      expect(model.items()[0]?.read).toBe(true);
      await markPromise;

      expect(model.items()[0]?.read).toBe(false);
      expect(model.actionError()).toBe(errorText('roleNotAllowed', 'RoleNotAllowed'));

      // Stale refetch (#2) resolves late with read = true -> must not overwrite the rolled-back item
      staleRefetchReq.resolve(
        jsonResponse({
          version: 1,
          changed: true,
          data: { items: [{ ...UNREAD_ITEM, read: true }] },
        }),
      );
      await stalePromise;
      await flush();

      expect(model.items()[0]?.read).toBe(false);
      expect(model.actionError()).toBe(errorText('roleNotAllowed', 'RoleNotAllowed'));
      dispose();
    });
  });

  it('silent read superseding an in-flight toggle read clears loading() and surfaces errors', async () => {
    let viewCall = 0;
    const toggleRead1 = deferred<Response>();
    const silentRead1 = deferred<Response>();
    const toggleRead2 = deferred<Response>();
    const silentRead2 = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [UNREAD_ITEM, READ_ITEM] } }),
          );
        }
        if (viewCall === 2) return toggleRead1.promise;
        if (viewCall === 3) return silentRead1.promise;
        if (viewCall === 4) return toggleRead2.promise;
        return silentRead2.promise;
      }

      if (url.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        return Promise.resolve(jsonResponse({ updatedCount: 1 }));
      }

      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flush();
      expect(model.loading()).toBe(false);

      // 1. Toggle unreadOnly -> non-silent read #2 in flight, loading() is true
      model.setUnreadOnly(true);
      await flush();
      expect(model.loading()).toBe(true);
      expect(viewCall).toBe(2);

      // 2. markAllRead() -> POST resolves and starts silent read #3 while toggle read #2 is still out
      const markAllPromise1 = model.markAllRead();
      await flush();
      expect(viewCall).toBe(3);
      expect(model.loading()).toBe(true);

      // Toggle read #2 lands first while silent read #3 is still out -> loading() stays true
      toggleRead1.resolve(
        jsonResponse({ version: 2, changed: true, data: { items: [UNREAD_ITEM] } }),
      );
      await flush();
      expect(model.loading()).toBe(true);

      // Silent read #3 lands -> must clear loading() to false
      silentRead1.resolve(
        jsonResponse({ version: 3, changed: true, data: { items: [] } }),
      );
      await markAllPromise1;
      await flush();
      expect(model.loading()).toBe(false);
      expect(model.items()).toEqual([]);

      // 3. Toggle unreadOnly(false) -> non-silent read #4 in flight, superseded by silent read #5 that errors
      model.setUnreadOnly(false);
      await flush();
      expect(model.loading()).toBe(true);
      expect(viewCall).toBe(4);

      const markAllPromise2 = model.markAllRead();
      await flush();
      expect(viewCall).toBe(5);

      toggleRead2.resolve(
        jsonResponse({ version: 4, changed: true, data: { items: [READ_ITEM] } }),
      );
      await flush();
      expect(model.loading()).toBe(true);

      silentRead2.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'Boom' } }, 500),
      );
      await markAllPromise2;
      await flush();
      expect(model.loading()).toBe(false);
      expect(model.loadError()).toBe(errorText('serverError', 'Boom'));

      dispose();
    });
  });
});

