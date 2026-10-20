import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  type MyNotificationItem,
  makeMyNotificationItem,
  makeMyNotificationObject,
} from '../../../shared/testing/fixtures';
import { NotificationsScreen, objectHref } from './Notifications';
import * as table from './i18n';

describe('C13: Notification object links (#3322)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: { method: string; path: string }[] = [];
  let viewItems: MyNotificationItem[];

  beforeEach(() => {
    setLang('en');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    viewItems = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      sentRequests.push({ method, path: u.pathname });

      if (u.pathname === '/api/v3/views/my_notifications') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: { items: viewItems } }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname.startsWith('/api/app/notifications/') && u.pathname.endsWith('/read') && method === 'POST') {
        return new Response(
          JSON.stringify({ readAt: '2026-10-19T00:00:00Z' }),
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

  it('open key present and non-empty in th/en/zh', () => {
    expect(table.th.open).toBeDefined();
    expect(typeof table.th.open).toBe('string');
    expect(table.th.open.trim().length).toBeGreaterThan(0);

    expect(table.en.open).toBeDefined();
    expect(typeof table.en.open).toBe('string');
    expect(table.en.open.trim().length).toBeGreaterThan(0);

    expect(table.zh.open).toBeDefined();
    expect(typeof table.zh.open).toBe('string');
    expect(table.zh.open.trim().length).toBeGreaterThan(0);
  });

  it('objectHref maps known object types correctly and returns undefined for unknown/empty', () => {
    expect(objectHref({ objectType: 'rate_alert_event', objectId: 'alert-1' })).toBe('/app/rate-alerts');
    expect(objectHref({ objectType: 'rate_publication', objectId: 'pub-1' })).toBe('/rates');
    expect(objectHref({ objectType: 'queue_ticket', objectId: 'q-1' })).toBeUndefined();
    expect(objectHref(undefined)).toBeUndefined();
  });

  it('rate_alert_event renders link href="/app/rate-alerts" with open text', async () => {
    setLang('en');
    viewItems = [
      makeMyNotificationItem({
        id: 'n1',
        seq: 1,
        locale: 'en',
        title: 'Rate Alert',
        body: 'USD/THB crossed target',
        object: makeMyNotificationObject({ objectType: 'rate_alert_event', objectId: 'alert-1' }),
        read: false,
        createdAt: '2026-10-19T08:00:00Z',
      }),
    ];

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const link = container.querySelector('a.xc-notifications-open') as HTMLAnchorElement | null;
    expect(link).not.toBeNull();
    expect(link?.getAttribute('href')).toBe('/app/rate-alerts');
    expect(link?.textContent?.trim()).toBe(table.en.open);
  });

  it('rate_publication renders link href="/rates"', async () => {
    viewItems = [
      makeMyNotificationItem({
        id: 'n2',
        seq: 2,
        locale: 'en',
        title: 'Rates updated',
        body: 'New rates published',
        object: makeMyNotificationObject({ objectType: 'rate_publication', objectId: 'pub-1' }),
        read: true,
        createdAt: '2026-10-19T08:00:00Z',
      }),
    ];

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const link = container.querySelector('a.xc-notifications-open') as HTMLAnchorElement | null;
    expect(link).not.toBeNull();
    expect(link?.getAttribute('href')).toBe('/rates');
  });

  it('queue_ticket (unknown) renders no link', async () => {
    viewItems = [
      makeMyNotificationItem({
        id: 'n3',
        seq: 3,
        locale: 'en',
        title: 'Queue update',
        body: 'Queue #42 ready',
        object: makeMyNotificationObject({ objectType: 'queue_ticket', objectId: 'ticket-42' }),
        read: false,
        createdAt: '2026-10-19T08:00:00Z',
      }),
    ];

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const link = container.querySelector('a.xc-notifications-open');
    expect(link).toBeNull();
  });

  it('item with no object renders no link', async () => {
    viewItems = [
      makeMyNotificationItem({
        id: 'n4',
        seq: 4,
        locale: 'en',
        title: 'System message',
        body: 'Maintenance tonight',
        read: false,
        createdAt: '2026-10-19T08:00:00Z',
      }),
    ];

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const link = container.querySelector('a.xc-notifications-open');
    expect(link).toBeNull();
  });

  it('clicking link on unread item does NOT trigger notificationRead call', async () => {
    viewItems = [
      makeMyNotificationItem({
        id: 'n5',
        seq: 5,
        locale: 'en',
        title: 'Alert',
        body: 'Check alerts',
        object: makeMyNotificationObject({ objectType: 'rate_alert_event', objectId: 'alert-5' }),
        read: false,
        createdAt: '2026-10-19T08:00:00Z',
      }),
    ];

    dispose = render(() => <NotificationsScreen />, container);
    await flushPromises();

    const link = container.querySelector('a.xc-notifications-open') as HTMLAnchorElement | null;
    expect(link).not.toBeNull();

    link?.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
    await flushPromises();

    const readRequests = sentRequests.filter(
      (r) => r.path.startsWith('/api/app/notifications/') && r.path.endsWith('/read'),
    );
    expect(readRequests.length).toBe(0);
  });
});
