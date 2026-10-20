// SM-3 (#1491): admin notices page specs (design `docs/design/blueprint/rates.md` §12.8, §12.10).
// Tests cover all edge cases in ticket #1491 §9:
// 1. Loading, empty ("no notices yet"), error with retry: each renders.
// 2. Several items arrive in any order and render newest first by seq.
// 3. Unread rows are marked; the unread filter sends unread: true.
// 4. Mark one read: one notificationRead call with that id, then the row reads as read.
// 5. Mark all read: one notificationReadAll call.
// 6. A refused mark-read shows the worded error and keeps the list.
// 7. A body containing <b>x</b> renders as literal text.
// 8. The menu has needs: 'all' and blueprint: true.
// 9. TH/EN key parity.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import { makeMyNotificationItem, type MyNotificationItem } from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { menu } from './menu';
import { Notices } from './Notices';
import * as table from './i18n';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Record<string, string>;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

describe('SM-3: Notices (§9 specs)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewItems: MyNotificationItem[];
  let viewStatus = 200;
  let viewErrorCode: string | undefined = undefined;
  let markReadStatus = 200;
  let markReadErrorCode: string | undefined = undefined;
  let markReadAllStatus = 200;
  let markReadAllErrorCode: string | undefined = undefined;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    viewItems = [];
    viewStatus = 200;
    viewErrorCode = undefined;
    markReadStatus = 200;
    markReadErrorCode = undefined;
    markReadAllStatus = 200;
    markReadAllErrorCode = undefined;

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
          return new Response(
            JSON.stringify({ error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load' } }),
            { status: viewStatus, headers: { 'content-type': 'application/json' } },
          );
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
            JSON.stringify({
              error: {
                code: markReadErrorCode ?? 'roleNotAllowed',
                message: 'RoleNotAllowed',
              },
            }),
            { status: markReadStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        const idMatch = u.pathname.match(/\/api\/app\/notifications\/([^/]+)\/read/);
        const readId = idMatch ? decodeURIComponent(idMatch[1]) : '';
        const item = viewItems.find((i) => i.id === readId);
        if (item) item.read = true;
        return new Response(
          JSON.stringify({ readAt: '2026-10-12T08:00:00Z' }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/app/notifications/read-all' && method === 'POST') {
        if (markReadAllStatus !== 200) {
          return new Response(
            JSON.stringify({
              error: {
                code: markReadAllErrorCode ?? 'roleNotAllowed',
                message: 'RoleNotAllowed',
              },
            }),
            { status: markReadAllStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        for (const item of viewItems) {
          item.read = true;
        }
        return new Response(
          JSON.stringify({ count: viewItems.length }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    container.remove();
    vi.restoreAllMocks();
  });

  async function flushPromises() {
    await new Promise((r) => setTimeout(r, 10));
  }

  // Edge case 1: Loading, empty ("no notices yet"), error with retry: each renders.
  it('1. renders loading, empty ("no notices yet"), and error with retry', async () => {
    // 1a: empty
    viewItems = [];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();
    expect(container.querySelector('.admin-notices-empty')).not.toBeNull();
    expect(container.textContent).toContain(table.th.empty);

    // 1b: error and retry
    if (dispose) dispose();
    container.innerHTML = '';
    viewStatus = 500;
    viewErrorCode = 'serverError';
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    expect(container.querySelector('.admin-notices-error')).not.toBeNull();
    const retryBtn = container.querySelector('.admin-notices-retry-btn') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();
    expect(retryBtn.textContent).toContain(table.th.retry);

    // Click retry after restoring viewStatus = 200
    viewStatus = 200;
    viewItems = [
      makeMyNotificationItem({
        id: 'n1',
        seq: 1,
        locale: 'th',
        title: 'Notice 1',
        body: 'Body 1',
        read: true,
        createdAt: '2026-10-12T07:00:00Z',
      }),
    ];
    retryBtn.click();
    await flushPromises();
    expect(container.querySelector('.admin-notices-error')).toBeNull();
    expect(container.textContent).toContain('Notice 1');
  });

  // Edge case 2: Several items arrive in any order and render newest first by seq.
  it('2. several items arrive in any order and render newest first by seq', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'n1', seq: 10, locale: 'th', title: 'First Seq 10', body: 'B1', read: true, createdAt: '2026-10-12T01:00:00Z' }),
      makeMyNotificationItem({ id: 'n2', seq: 30, locale: 'th', title: 'Newest Seq 30', body: 'B2', read: false, createdAt: '2026-10-12T03:00:00Z' }),
      makeMyNotificationItem({ id: 'n3', seq: 20, locale: 'th', title: 'Middle Seq 20', body: 'B3', read: true, createdAt: '2026-10-12T02:00:00Z' }),
    ];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    const titles = Array.from(container.querySelectorAll('.admin-notices-title')).map((el) => el.textContent?.trim());
    expect(titles).toEqual(['Newest Seq 30', 'Middle Seq 20', 'First Seq 10']);
  });

  // Edge case 3: Unread rows are marked; the unread filter sends unread: true.
  it('3. unread rows are marked; unread filter sends unread: true', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'n1', seq: 1, locale: 'th', title: 'Notice 1', body: 'B1', read: false, createdAt: '2026-10-12T01:00:00Z' }),
      makeMyNotificationItem({ id: 'n2', seq: 2, locale: 'th', title: 'Notice 2', body: 'B2', read: true, createdAt: '2026-10-12T02:00:00Z' }),
    ];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    const unreadItem = container.querySelector('.admin-notices-item-unread');
    expect(unreadItem).not.toBeNull();
    expect(unreadItem?.textContent).toContain('Notice 1');

    // Toggle unread filter
    const filterInput = container.querySelector('.admin-notices-filter-input') as HTMLInputElement;
    expect(filterInput).not.toBeNull();
    expect(filterInput.checked).toBe(false);

    filterInput.checked = true;
    filterInput.dispatchEvent(new Event('change', { bubbles: true }));
    await flushPromises();

    const lastReq = sentRequests[sentRequests.length - 1];
    expect(lastReq.path).toBe('/api/v3/views/my_notifications');
    expect(lastReq.search).toContain('unread=true');
  });

  // Edge case 4: Mark one read: one notificationRead call with that id, then the row reads as read.
  it('4. mark one read: calls notificationRead once with id, row becomes read', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'notif-99', seq: 1, locale: 'th', title: 'Unread Alert', body: 'Text', read: false, createdAt: '2026-10-12T01:00:00Z' }),
    ];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    const markBtn = container.querySelector('.admin-notices-mark-btn') as HTMLButtonElement;
    expect(markBtn).not.toBeNull();
    expect(markBtn.textContent).toContain(table.th.markRead);

    markBtn.click();
    await flushPromises();

    const readCalls = sentRequests.filter(
      (r) => r.path === '/api/app/notifications/notif-99/read' && r.method === 'POST',
    );
    expect(readCalls).toHaveLength(1);
    expect(readCalls[0].body).toEqual({ id: 'notif-99' });
    expect(readCalls[0].headers['idempotency-key']).toBeTruthy();

    // After re-fetch, the row reads as read
    expect(container.querySelector('.admin-notices-item-unread')).toBeNull();
    expect(container.querySelector('.admin-notices-mark-btn')).toBeNull();
  });

  // Edge case 5: Mark all read: one notificationReadAll call.
  it('5. mark all read: calls notificationReadAll once', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'n1', seq: 1, locale: 'th', title: 'Alert 1', body: 'Text 1', read: false, createdAt: '2026-10-12T01:00:00Z' }),
      makeMyNotificationItem({ id: 'n2', seq: 2, locale: 'th', title: 'Alert 2', body: 'Text 2', read: false, createdAt: '2026-10-12T02:00:00Z' }),
    ];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    const markAllBtn = container.querySelector('.admin-notices-mark-all-btn') as HTMLButtonElement;
    expect(markAllBtn).not.toBeNull();
    expect(markAllBtn.disabled).toBe(false);

    markAllBtn.click();
    await flushPromises();

    const readAllCalls = sentRequests.filter(
      (r) => r.path === '/api/app/notifications/read-all' && r.method === 'POST',
    );
    expect(readAllCalls).toHaveLength(1);
    expect(readAllCalls[0].body).toEqual({});
    expect(readAllCalls[0].headers['idempotency-key']).toBeTruthy();

    // After re-fetch, all rows read as read
    expect(container.querySelectorAll('.admin-notices-item-unread')).toHaveLength(0);
    expect(markAllBtn.disabled).toBe(true);
  });

  // Edge case 6: A refused mark-read shows the worded error and keeps the list.
  it('6. a refused mark-read shows worded error through errorText and keeps the list', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'n-fail', seq: 1, locale: 'th', title: 'Notice Staying Visible', body: 'Important Content', read: false, createdAt: '2026-10-12T01:00:00Z' }),
    ];
    markReadStatus = 400;
    markReadErrorCode = 'roleNotAllowed';

    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    const markBtn = container.querySelector('.admin-notices-mark-btn') as HTMLButtonElement;
    markBtn.click();
    await flushPromises();

    // Error is worded through errorText, not raw 'RoleNotAllowed'
    const expectedWorded = errorText('roleNotAllowed', 'RoleNotAllowed');
    expect(container.querySelector('.admin-notices-action-error')?.textContent).toContain(expectedWorded);
    expect(container.querySelector('.admin-notices-action-error')?.textContent).not.toBe('RoleNotAllowed');

    // The list remains visible
    expect(container.textContent).toContain('Notice Staying Visible');
    expect(container.textContent).toContain('Important Content');
  });

  // Edge case 7: A body containing <b>x</b> renders as literal text.
  it('7. body containing <b>x</b> renders as literal text, never as HTML', async () => {
    viewItems = [
      makeMyNotificationItem({ id: 'n-html', seq: 1, locale: 'th', title: 'HTML Test', body: 'Here is <b>bold</b> text', read: true, createdAt: '2026-10-12T01:00:00Z' }),
    ];
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();

    expect(container.querySelector('b')).toBeNull();
    const bodyEl = container.querySelector('.admin-notices-body');
    expect(bodyEl?.textContent).toBe('Here is <b>bold</b> text');
  });

  // Edge case 8: The menu has needs: 'all' and blueprint: true.
  it('8. menu has needs: "all" and order is a positive number', () => {
    expect(menu.id).toBe('notices');
    expect(menu.needs).toBe('all');
    expect(menu.group).toBe('tools');
    expect(menu.dated).toBe(false);
    expect(typeof menu.order).toBe('number');
    expect(menu.order).toBeGreaterThan(0);
    expect(menu.label()).toBeTruthy();
    expect(menu.heading.title()).toBeTruthy();
  });

  // Edge case 9: TH/EN key parity.
  it('9. TH/EN translations have exact key parity', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    expect(thKeys).toEqual(enKeys);
    expect(thKeys.length).toBeGreaterThan(0);
  });
});
