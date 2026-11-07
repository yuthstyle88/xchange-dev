// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// Do not edit, delete or weaken these tests.
//
// A notification's time is an instant a person reads, so it is shown with formatBangkokDateTime
// (no seconds, CE year), not toLocaleString(locale(), { timeZone: 'Asia/Bangkok' }) (`4/11/2569 13:23:45`).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeMyNotificationItem } from '../../../shared/testing/fixtures';
import { NotificationsScreen } from './Notifications';

let container: HTMLDivElement;
let dispose: (() => void) | null = null;
let viewItems = [] as ReturnType<typeof makeMyNotificationItem>[];

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  // 06:23:45Z = 13:23 ICT on 04/11; 17:05Z on 04/11 = 00:05 ICT on 05/11.
  viewItems = [
    makeMyNotificationItem({ id: 'n1', seq: 2, locale: 'th', title: 'A', body: 'a', read: false, createdAt: '2026-11-04T06:23:45Z' }),
    makeMyNotificationItem({ id: 'n2', seq: 1, locale: 'th', title: 'B', body: 'b', read: true, createdAt: '2026-11-04T17:05:00Z' }),
  ];
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (new URL(url, 'http://localhost').pathname === '/api/v3/views/my_notifications') {
      return new Response(JSON.stringify({ version: 1, changed: true, data: { items: viewItems } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
  });
});

afterEach(() => {
  dispose?.();
  dispose = null;
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

async function times(): Promise<string[]> {
  dispose = render(() => <NotificationsScreen />, container);
  await new Promise((r) => setTimeout(r, 10));
  return Array.from(container.querySelectorAll('.xc-notifications-time')).map((e) => e.textContent ?? '');
}

describe('F55 Notifications time', () => {
  it('Thai: each time reads dd/mm/yyyy HH:mm, CE year, Bangkok, no seconds', async () => {
    setLang('th');
    expect((await times()).sort()).toEqual(['04/11/2026 13:23', '05/11/2026 00:05']);
  });

  it('English: the same text', async () => {
    setLang('en');
    expect((await times()).sort()).toEqual(['04/11/2026 13:23', '05/11/2026 00:05']);
  });
});
