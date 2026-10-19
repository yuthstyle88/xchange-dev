import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../api/client';
import { errorText } from '../i18n';
import { createNotifications } from './notifications';
import { makeMyNotificationItem, type MyNotificationItem } from '../testing/fixtures';

describe('createNotifications model', () => {
  let sentRequests: { method: string; path: string; search: string; body: unknown; headers: Record<string, string> }[] = [];
  let viewItems: MyNotificationItem[];
  let viewStatus = 200;
  let markReadStatus = 200;
  let markReadAllStatus = 200;

  beforeEach(() => {
    sentRequests = [];
    viewItems = [];
    viewStatus = 200;
    markReadStatus = 200;
    markReadAllStatus = 200;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body, headers });

      if (u.pathname === '/api/v3/views/my_notifications') {
        if (viewStatus !== 200) {
          return new Response(JSON.stringify({ error: { code: 'serverError', message: 'Failed' } }), {
            status: viewStatus,
            headers: { 'content-type': 'application/json' },
          });
        }
        const unreadParam = u.searchParams.get('unread');
        let filtered = viewItems;
        if (unreadParam === 'true') {
          filtered = viewItems.filter((i) => !i.read);
        }
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: { items: filtered } }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname.startsWith('/api/app/notifications/') && u.pathname.endsWith('/read') && method === 'POST') {
        if (markReadStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: 'roleNotAllowed', message: 'RoleNotAllowed' } }),
            { status: markReadStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        const idMatch = u.pathname.match(/\/api\/app\/notifications\/([^/]+)\/read/);
        const readId = idMatch ? decodeURIComponent(idMatch[1]) : '';
        const item = viewItems.find((i) => i.id === readId);
        if (item) item.read = true;
        return new Response(JSON.stringify({ readAt: '2026-10-13T12:00:00Z' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (u.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        if (markReadAllStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: 'serverError', message: 'Failed' } }),
            { status: markReadAllStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        for (const item of viewItems) {
          item.read = true;
        }
        return new Response(JSON.stringify({ count: viewItems.length }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  async function flushPromises() {
    await new Promise((r) => setTimeout(r, 10));
  }

  it('loads items and sorts newest first by seq', async () => {
    viewItems = [
      makeMyNotificationItem({ id: '1', seq: 5, locale: 'th', title: 'Older', body: 'b1', read: false, createdAt: '2026-10-13T01:00:00Z' }),
      makeMyNotificationItem({ id: '2', seq: 20, locale: 'th', title: 'Newest', body: 'b2', read: false, createdAt: '2026-10-13T02:00:00Z' }),
      makeMyNotificationItem({ id: '3', seq: 12, locale: 'th', title: 'Middle', body: 'b3', read: true, createdAt: '2026-10-13T01:30:00Z' }),
    ];

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      expect(model.loading()).toBe(true);
      await flushPromises();

      expect(model.loading()).toBe(false);
      expect(model.loadError()).toBeNull();
      expect(model.items().map((i) => i.seq)).toEqual([20, 12, 5]);
      dispose();
    });
  });

  it('filters unread items when unreadOnly is toggled', async () => {
    viewItems = [
      makeMyNotificationItem({ id: '1', seq: 1, locale: 'th', title: 'Unread', body: 'b1', read: false, createdAt: '2026-10-13T01:00:00Z' }),
      makeMyNotificationItem({ id: '2', seq: 2, locale: 'th', title: 'Read', body: 'b2', read: true, createdAt: '2026-10-13T02:00:00Z' }),
    ];

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient());
      await flushPromises();
      expect(model.items()).toHaveLength(2);

      model.setUnreadOnly(true);
      await flushPromises();

      const lastReq = sentRequests[sentRequests.length - 1];
      expect(lastReq.search).toContain('unread=true');
      expect(model.items()).toHaveLength(1);
      expect(model.items()[0].id).toBe('1');
      dispose();
    });
  });

  it('marks one read with optimistic: true updates immediately and keeps read after refetch', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'item-1', seq: 1, locale: 'th', title: 'Notice', body: 'text', read: false, createdAt: '2026-10-13T01:00:00Z' }),
    ];

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flushPromises();
      expect(model.items()[0].read).toBe(false);

      const promise = model.markOneRead('item-1');
      // Optimistically marked read immediately
      expect(model.items()[0].read).toBe(true);
      expect(model.markingIds().has('item-1')).toBe(true);

      await promise;
      expect(model.items()[0].read).toBe(true);
      expect(model.markingIds().has('item-1')).toBe(false);

      const readReq = sentRequests.find((r) => r.path === '/api/app/notifications/item-1/read');
      expect(readReq).toBeTruthy();
      expect(readReq?.headers['idempotency-key']).toBeTruthy();
      dispose();
    });
  });

  it('rolls back optimistic markOneRead on server failure and shows worded error', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'item-fail', seq: 1, locale: 'th', title: 'Notice', body: 'text', read: false, createdAt: '2026-10-13T01:00:00Z' }),
    ];
    markReadStatus = 403;

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flushPromises();
      expect(model.items()[0].read).toBe(false);

      const promise = model.markOneRead('item-fail');
      // Optimistically true before failure
      expect(model.items()[0].read).toBe(true);

      await promise;
      // Rolled back to false
      expect(model.items()[0].read).toBe(false);
      expect(model.actionError()).toBe(errorText('roleNotAllowed', 'RoleNotAllowed'));
      dispose();
    });
  });

  it('marks all read calls /api/app/notifications/read-all with idempotency key', async () => {
    viewItems = [
      makeMyNotificationItem({ id: '1', seq: 1, locale: 'th', title: 'N1', body: 'b1', read: false, createdAt: '2026-10-13T01:00:00Z' }),
      makeMyNotificationItem({ id: '2', seq: 2, locale: 'th', title: 'N2', body: 'b2', read: false, createdAt: '2026-10-13T02:00:00Z' }),
    ];

    await createRoot(async (dispose) => {
      const model = createNotifications(new ApiClient(), { optimistic: true });
      await flushPromises();

      await model.markAllRead();
      expect(model.items().every((i) => i.read)).toBe(true);

      const readAllReq = sentRequests.find((r) => r.path === '/api/app/notifications/read-all');
      expect(readAllReq).toBeTruthy();
      expect(readAllReq?.headers['idempotency-key']).toBeTruthy();
      dispose();
    });
  });
});
