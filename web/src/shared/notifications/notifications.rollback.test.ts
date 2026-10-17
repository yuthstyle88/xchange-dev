import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../api/client';
import { errorText } from '../i18n';
import type { MyNotificationItem } from '../api/generated/my_notifications';
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

const ITEM_A: MyNotificationItem = {
  id: 'a',
  seq: 30,
  locale: 'th',
  title: 'Notice A',
  body: 'body a',
  read: false,
  createdAt: '2026-10-13T03:00:00Z',
};

const ITEM_B: MyNotificationItem = {
  id: 'b',
  seq: 20,
  locale: 'th',
  title: 'Notice B',
  body: 'body b',
  read: false,
  createdAt: '2026-10-13T02:00:00Z',
};

const ITEM_C: MyNotificationItem = {
  id: 'c',
  seq: 10,
  locale: 'th',
  title: 'Notice C',
  body: 'body c',
  read: true,
  createdAt: '2026-10-13T01:00:00Z',
};

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
  }
}

describe('createNotifications optimistic rollback (#2402)', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('row 1: Mark A, mark B; A fails, then B fails -> Both unread; A is never shown read', async () => {
    const markAReq = deferred<Response>();
    const markBReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A, ITEM_B] } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      if (url.pathname === '/api/app/notifications/b/read' && method === 'POST') {
        return markBReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      const pA = model.markOneRead('a');
      const pB = model.markOneRead('b');
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);

      // A fails first -> A becomes unread while B (still in flight) stays optimistically read
      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(false);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);

      // B fails second -> both A and B are unread; A is never flipped back to read
      markBReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'B failed' } }, 500),
      );
      await pB;
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(false);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(false);
      dispose();
    });
  });

  it('row 2: Mark A, mark B; B succeeds and its refetch lands; A fails -> B stays read, A unread', async () => {
    const markAReq = deferred<Response>();
    const markBReq = deferred<Response>();
    let viewCall = 0;

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A, ITEM_B] } }),
          );
        }
        return Promise.resolve(
          jsonResponse({
            version: 2,
            changed: true,
            data: { items: [ITEM_A, { ...ITEM_B, read: true }] },
          }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      if (url.pathname === '/api/app/notifications/b/read' && method === 'POST') {
        return markBReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      const pA = model.markOneRead('a');
      const pB = model.markOneRead('b');

      // B succeeds and its silent refetch lands
      markBReq.resolve(jsonResponse({ readAt: '2026-10-13T12:00:00Z' }));
      await pB;
      await flush();
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);

      // A fails afterwards -> B must stay read, A must be unread
      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(false);
      dispose();
    });
  });

  it('row 3: Mark A; toggle unread-only; the unread list lands; A fails -> The unread list stays, with A unread', async () => {
    const markAReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        if (url.searchParams.get('unread') === 'true') {
          return Promise.resolve(
            jsonResponse({ version: 2, changed: true, data: { items: [ITEM_A] } }),
          );
        }
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A, ITEM_C] } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['a', 'c']);

      // Mark A while viewing full list
      const pA = model.markOneRead('a');
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);

      // Toggle unread-only -> unread list ([ITEM_A]) lands
      model.setUnreadOnly(true);
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['a']);

      // A fails -> unread list stays ([ITEM_A] only, ITEM_C does not reappear), with A unread
      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;
      expect(model.items().map((i) => i.id)).toEqual(['a']);
      expect(model.items()[0]?.read).toBe(false);
      dispose();
    });
  });

  it('row 4: One mark fails -> Rolled back with a worded error without clobbering newer items', async () => {
    const markAReq = deferred<Response>();
    let viewCall = 0;

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A] } }),
          );
        }
        return Promise.resolve(
          jsonResponse({ version: 2, changed: true, data: { items: [ITEM_A, ITEM_B] } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      const pA = model.markOneRead('a');
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);

      // A refetch lands while mark A is in flight
      await model.refetch();
      expect(model.items().map((i) => i.id)).toEqual(['a', 'b']);

      markAReq.resolve(
        jsonResponse({ error: { code: 'roleNotAllowed', message: 'RoleNotAllowed' } }, 403),
      );
      await pA;

      expect(model.items().find((i) => i.id === 'a')?.read).toBe(false);
      expect(model.items().map((i) => i.id)).toEqual(['a', 'b']);
      expect(model.actionError()).toBe(errorText('roleNotAllowed', 'RoleNotAllowed'));
      dispose();
    });
  });

  it('row 5: One mark succeeds -> Stays read after the refetch even when an earlier concurrent mark fails', async () => {
    const markAReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        return Promise.resolve(
          jsonResponse({
            version: 1,
            changed: true,
            data: { items: [ITEM_A, { ...ITEM_B, read: markBCompleted }] },
          }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      if (url.pathname === '/api/app/notifications/b/read' && method === 'POST') {
        markBCompleted = true;
        return Promise.resolve(jsonResponse({ readAt: '2026-10-13T12:00:00Z' }));
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    let markBCompleted = false;

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      const pA = model.markOneRead('a');
      await model.markOneRead('b');
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);

      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;

      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);
      dispose();
    });
  });

  it("edge case 1: markAll fails while a single mark is in flight: only markAll's ids revert", async () => {
    const markAReq = deferred<Response>();
    const markAllReq = deferred<Response>();

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A, ITEM_B, ITEM_C] } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      if (url.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        return markAllReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      // Start markAllRead (flips A and B to read) and markOneRead('a') while markAll is in flight
      const pAll = model.markAllRead();
      const pA = model.markOneRead('a');
      expect(model.items().every((i) => i.read)).toBe(true);

      // markAll fails while single mark on A is still in flight -> only B reverts; A and already-read C stay read
      markAllReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'markAll failed' } }, 500),
      );
      await pAll;

      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(false);
      expect(model.items().find((i) => i.id === 'c')?.read).toBe(true);

      // Finish single mark on A
      markAReq.resolve(jsonResponse({ readAt: '2026-10-13T12:00:00Z' }));
      await pA;
      dispose();
    });
  });

  it('edge case 2: A mark on an id a newer load no longer lists: the revert is a no-op', async () => {
    const markAReq = deferred<Response>();
    let viewCall = 0;

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        viewCall += 1;
        if (viewCall === 1) {
          return Promise.resolve(
            jsonResponse({ version: 1, changed: true, data: { items: [ITEM_A, ITEM_B] } }),
          );
        }
        // Newer load no longer lists ITEM_A
        return Promise.resolve(
          jsonResponse({ version: 2, changed: true, data: { items: [ITEM_B] } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['a', 'b']);

      const pA = model.markOneRead('a');
      await model.refetch();
      expect(model.items().map((i) => i.id)).toEqual(['b']);

      // Mark A fails after newer load dropped A -> revert is a no-op, A is not re-added
      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;

      expect(model.items().map((i) => i.id)).toEqual(['b']);
      expect(model.items()[0]?.read).toBe(false);
      dispose();
    });
  });

  it('P1: markOne fails after a successful markAll whose refetch landed -> item stays read', async () => {
    const markAReq = deferred<Response>();
    let allMarked = false;

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        return Promise.resolve(
          jsonResponse({
            version: allMarked ? 2 : 1,
            changed: true,
            data: {
              items: [
                { ...ITEM_A, read: allMarked },
                { ...ITEM_B, read: allMarked },
              ],
            },
          }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        return markAReq.promise;
      }
      if (url.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        allMarked = true;
        return Promise.resolve(jsonResponse({ updatedCount: 2 }));
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flush();

      // Tap "mark read" on A (slow), then "mark all read" while B is still unread
      const pA = model.markOneRead('a');
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(false);

      // markAll succeeds and its refetch lands with A and B read
      await model.markAllRead();
      await flush();
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);

      // Now A's slow write fails -> A must stay read (markAll's newer read stands)
      markAReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'A failed' } }, 500),
      );
      await pA;

      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(true);
      dispose();
    });
  });

  it('P2: markAll fails after a single mark on a re-listed id succeeded -> that id stays read', async () => {
    const markAllReq = deferred<Response>();
    let aMarked = false;

    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.pathname === '/api/v3/views/my_notifications' && method === 'GET') {
        const current = [
          { ...ITEM_A, read: aMarked },
          { ...ITEM_B, read: false },
          ITEM_C,
        ];
        const items =
          url.searchParams.get('unread') === 'true'
            ? current.filter((i) => !i.read)
            : current;
        return Promise.resolve(
          jsonResponse({ version: aMarked ? 2 : 1, changed: true, data: { items } }),
        );
      }
      if (url.pathname === '/api/app/notifications/a/read' && method === 'POST') {
        aMarked = true;
        return Promise.resolve(jsonResponse({ readAt: '2026-10-13T12:00:00Z' }));
      }
      if (url.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        return markAllReq.promise;
      }
      return Promise.resolve(jsonResponse({ error: 'not found' }, 404));
    });

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), {
        optimistic: true,
        initialUnreadOnly: true,
      });
      await flush();
      expect(model.items().map((i) => i.id)).toEqual(['a', 'b']);

      // Tap "mark all read" (slow) while unread-only is on
      const pAll = model.markAllRead();
      expect(model.items().every((i) => i.read)).toBe(true);

      // Toggle filter off -> full list lands with A and B unread
      model.setUnreadOnly(false);
      await flush();
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(false);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(false);

      // Mark A, which succeeds, and its refetch shows A read
      await model.markOneRead('a');
      await flush();
      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);

      // Now slow markAll fails -> A must stay read
      markAllReq.resolve(
        jsonResponse({ error: { code: 'serverError', message: 'markAll failed' } }, 500),
      );
      await pAll;

      expect(model.items().find((i) => i.id === 'a')?.read).toBe(true);
      expect(model.items().find((i) => i.id === 'b')?.read).toBe(false);
      expect(model.items().find((i) => i.id === 'c')?.read).toBe(true);
      dispose();
    });
  });
});

