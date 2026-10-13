import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { MyNotificationItem } from '../../../shared/api/generated/my_notifications';
import { NotificationsScreen } from './Notifications';
import * as table from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';
let fs: NodeFs;
let path: NodePath;

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Record<string, string>;
}

describe('APP-15: Customer NotificationsScreen (§9 specs)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewItems: MyNotificationItem[] = [];
  let viewStatus = 200;
  let viewErrorCode: string | undefined = undefined;
  let markReadStatus = 200;
  let markReadErrorCode: string | undefined = undefined;
  let markReadAllStatus = 200;
  let markReadAllErrorCode: string | undefined = undefined;
  let markReadDelay = 0;
  let viewDelayGate: Promise<void> | null = null;

  beforeAll(async () => {
    fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  });

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
    markReadDelay = 0;
    viewDelayGate = null;

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
        if (viewDelayGate) {
          await viewDelayGate;
        }
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
        if (markReadDelay > 0) {
          await new Promise((r) => setTimeout(r, markReadDelay));
        }
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
          JSON.stringify({ readAt: '2026-10-13T08:00:00Z' }),
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

  // Edge case 1: Loading, empty, and error with retry each render.
  it('1. renders loading, empty, and error with retry', async () => {
    // 1a: empty
    viewItems = [];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();
    expect(container.querySelector('.xc-notifications-empty')).not.toBeNull();
    expect(container.textContent).toContain(table.th.empty);

    // 1b: error and retry
    if (dispose) dispose();
    container.innerHTML = '';
    viewStatus = 500;
    viewErrorCode = 'serverError';
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    expect(container.querySelector('.xc-notifications-error')).not.toBeNull();
    const retryBtn = container.querySelector('.xc-notifications-retry-btn') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();
    expect(retryBtn.textContent).toContain(table.th.retry);

    // Click retry after restoring viewStatus = 200
    viewStatus = 200;
    viewItems = [
      {
        id: 'n1',
        seq: 1,
        locale: 'th',
        title: 'Customer Notice 1',
        body: 'Body text',
        read: true,
        createdAt: '2026-10-13T07:00:00Z',
      },
    ];
    retryBtn.click();
    await flushPromises();
    expect(container.querySelector('.xc-notifications-error')).toBeNull();
    expect(container.textContent).toContain('Customer Notice 1');
  });

  // Edge case 2: Items arriving in any order render newest first by seq.
  it('2. items arriving in any order render newest first by seq', async () => {
    viewItems = [
      { id: 'n1', seq: 5, locale: 'th', title: 'Seq 5', body: 'B1', read: true, createdAt: '2026-10-13T01:00:00Z' },
      { id: 'n2', seq: 50, locale: 'th', title: 'Seq 50 Newest', body: 'B2', read: false, createdAt: '2026-10-13T03:00:00Z' },
      { id: 'n3', seq: 12, locale: 'th', title: 'Seq 12 Middle', body: 'B3', read: true, createdAt: '2026-10-13T02:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const titles = Array.from(container.querySelectorAll('.xc-notifications-item-title')).map((el) => el.textContent?.trim());
    expect(titles).toEqual(['Seq 50 Newest', 'Seq 12 Middle', 'Seq 5']);
  });

  // Edge case 3: Unread rows are marked, and the unread filter sends unread: true.
  it('3. unread rows are marked; unread filter sends unread: true', async () => {
    viewItems = [
      { id: 'n1', seq: 1, locale: 'th', title: 'Notice 1', body: 'B1', read: false, createdAt: '2026-10-13T01:00:00Z' },
      { id: 'n2', seq: 2, locale: 'th', title: 'Notice 2', body: 'B2', read: true, createdAt: '2026-10-13T02:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const unreadItem = container.querySelector('.xc-notifications-item-unread');
    expect(unreadItem).not.toBeNull();
    expect(unreadItem?.textContent).toContain('Notice 1');

    const filterInput = container.querySelector('.xc-notifications-filter-input') as HTMLInputElement;
    expect(filterInput).not.toBeNull();
    expect(filterInput.checked).toBe(false);

    filterInput.checked = true;
    filterInput.dispatchEvent(new Event('change', { bubbles: true }));
    await flushPromises();

    const lastReq = sentRequests[sentRequests.length - 1];
    expect(lastReq.path).toBe('/api/v3/views/my_notifications');
    expect(lastReq.search).toContain('unread=true');
  });

  // Edge case 4: Mark one read is optimistic: the row reads as read at once, with one notificationRead {id} call.
  it('4. mark one read is optimistic: row reads as read at once with one notificationRead call', async () => {
    markReadDelay = 50; // Add delay so we can inspect optimistic state before server responds
    viewItems = [
      { id: 'item-opt-1', seq: 1, locale: 'th', title: 'Optimistic Alert', body: 'Body', read: false, createdAt: '2026-10-13T01:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const markBtn = container.querySelector('.xc-notifications-mark-btn') as HTMLButtonElement;
    expect(markBtn).not.toBeNull();
    expect(markBtn.textContent).toContain(table.th.markRead);

    // Trigger mark read
    markBtn.click();

    // Check optimistic update before delay finishes
    expect(container.querySelector('.xc-notifications-item-unread')).toBeNull();
    expect(container.querySelector('.xc-notifications-badge-read')).not.toBeNull();

    // Wait for network response and refetch to complete
    await new Promise((r) => setTimeout(r, 80));
    await flushPromises();

    const readCalls = sentRequests.filter(
      (r) => r.path === '/api/app/notifications/item-opt-1/read' && r.method === 'POST',
    );
    expect(readCalls).toHaveLength(1);
    expect(readCalls[0].body).toEqual({ id: 'item-opt-1' });
    expect(readCalls[0].headers['idempotency-key']).toBeTruthy();
    expect(container.querySelector('.xc-notifications-item-unread')).toBeNull();
  });

  // Edge case 5: A refused mark-read rolls the row back to unread and shows a worded error through errorText or your own i18n key, never err.message raw.
  it('5. refused mark-read rolls back to unread and shows worded error, never raw err.message', async () => {
    viewItems = [
      { id: 'item-fail', seq: 1, locale: 'th', title: 'Failing Notice', body: 'Important Content', read: false, createdAt: '2026-10-13T01:00:00Z' },
    ];
    markReadStatus = 400;
    markReadErrorCode = 'roleNotAllowed';

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const markBtn = container.querySelector('.xc-notifications-mark-btn') as HTMLButtonElement;
    markBtn.click();
    await flushPromises();

    // Rolled back to unread
    expect(container.querySelector('.xc-notifications-item-unread')).not.toBeNull();
    expect(container.querySelector('.xc-notifications-badge-unread')).not.toBeNull();

    // Error is worded, never raw 'RoleNotAllowed'
    const expectedWorded = errorText('roleNotAllowed', 'RoleNotAllowed');
    const errorEl = container.querySelector('.xc-notifications-action-error');
    expect(errorEl?.textContent).toContain(expectedWorded);
    expect(errorEl?.textContent).not.toBe('RoleNotAllowed');

    // List stays rendered
    expect(container.textContent).toContain('Failing Notice');
    expect(container.textContent).toContain('Important Content');
  });

  // Edge case 6: Mark all read makes one notificationReadAll call, and every row then reads as read.
  it('6. mark all read makes one notificationReadAll call and every row reads as read', async () => {
    viewItems = [
      { id: 'n1', seq: 1, locale: 'th', title: 'Alert 1', body: 'Text 1', read: false, createdAt: '2026-10-13T01:00:00Z' },
      { id: 'n2', seq: 2, locale: 'th', title: 'Alert 2', body: 'Text 2', read: false, createdAt: '2026-10-13T02:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const markAllBtn = container.querySelector('.xc-notifications-mark-all-btn') as HTMLButtonElement;
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

    expect(container.querySelectorAll('.xc-notifications-item-unread')).toHaveLength(0);
    expect(markAllBtn.disabled).toBe(true);
  });

  // Edge case 7: A body containing <b>x</b> renders as literal text.
  it('7. body containing <b>x</b> renders as literal text, never as HTML tags', async () => {
    viewItems = [
      { id: 'n-html', seq: 1, locale: 'th', title: 'HTML Safety', body: 'This is <b>escaped</b> text', read: true, createdAt: '2026-10-13T01:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    expect(container.querySelector('b')).toBeNull();
    const bodyEl = container.querySelector('.xc-notifications-body');
    expect(bodyEl?.textContent).toBe('This is <b>escaped</b> text');
  });

  // Edge case 8: main.tsx and app-notifications.html meet the shell and gate contract above.
  it('8. main.tsx and app-notifications.html meet shell and gate contract', () => {
    const thisFile = new URL(import.meta.url).pathname;
    const thisDir = path.dirname(thisFile);
    const webRoot = path.resolve(thisDir, '../../../../');
    const htmlPath = path.join(webRoot, 'pages/app-notifications.html');
    const mainPath = path.join(thisDir, 'main.tsx');

    expect(fs.existsSync(htmlPath), 'app-notifications.html exists').toBe(true);
    expect(fs.existsSync(mainPath), 'main.tsx exists').toBe(true);

    const htmlContent = fs.readFileSync(htmlPath, 'utf-8');
    expect(htmlContent).toContain('<title>XCHANGE — แจ้งเตือน | Notifications</title>');
    expect(htmlContent).toMatch(/meta\s+name=["']xc-gate["']\s+content=["']anyone["']/);
    expect(htmlContent).toMatch(/meta\s+name=["']xc-path["']\s+content=["']\/app\/notifications["']/);
    expect(htmlContent).toMatch(/<script\s+type=["']module["']\s+src=["']\/src\/screens\/customer\/notifications\/main\.tsx["']/);

    const mainContent = fs.readFileSync(mainPath, 'utf-8');
    expect(mainContent).toMatch(/setAudience\(['"]customer['"]\)/);
    expect(mainContent).toMatch(/import\s+\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s+['"][^'"]*_shell\/Shell['"]/);
    expect(mainContent).toMatch(/<CustomerShell\s+route=["']\/app\/notifications["']/);
  });

  // Edge case 9: TH/EN/ZH key parity.
  it('9. TH/EN/ZH translations have exact key parity', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    const zhKeys = Object.keys(table.zh).sort();

    expect(thKeys).toEqual(enKeys);
    expect(thKeys).toEqual(zhKeys);
    expect(thKeys.length).toBeGreaterThan(0);
  });

  // Post-action refresh is silent: notificationRead resolves while re-read stays pending.
  it('10. notificationRead resolves while re-read stays pending: list stays rendered, row reads as read, no loading text', async () => {
    viewItems = [
      { id: 'notif-pending-1', seq: 1, locale: 'th', title: 'Pending Notice', body: 'Body', read: false, createdAt: '2026-10-13T01:00:00Z' },
    ];
    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    expect(container.querySelector('.xc-notifications-list')).not.toBeNull();
    expect(container.querySelector('.xc-notifications-item-unread')).not.toBeNull();

    let releaseReRead!: () => void;
    viewDelayGate = new Promise<void>((resolve) => {
      releaseReRead = resolve;
    });

    const markBtn = container.querySelector('.xc-notifications-mark-btn') as HTMLButtonElement;
    expect(markBtn).not.toBeNull();
    markBtn.click();

    // Flush promises so the POST /read request finishes and refetch is initiated
    await flushPromises();

    // Verify POST /read was sent and completed
    const readCalls = sentRequests.filter(
      (r) => r.path === '/api/app/notifications/notif-pending-1/read' && r.method === 'POST',
    );
    expect(readCalls).toHaveLength(1);

    // While re-read is pending:
    // 1. List is still rendered
    expect(container.querySelector('.xc-notifications-list')).not.toBeNull();
    // 2. The row reads as read (not unread)
    expect(container.querySelector('.xc-notifications-item-unread')).toBeNull();
    expect(container.querySelector('.xc-notifications-badge-read')).not.toBeNull();
    // 3. No loading text is shown
    expect(container.querySelector('.xc-notifications-loading')).toBeNull();

    // Release the gate and ensure it resolves cleanly
    releaseReRead();
    await flushPromises();
    expect(container.querySelector('.xc-notifications-list')).not.toBeNull();
  });
});
