// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// A notice's createdAt is an instant, so its time reads dd/mm/yyyy HH:mm in both languages. Harness copied from
// `Notices.test.tsx`. Written before the implementation by the ticket author. Do not edit, delete or weaken; add tests
// in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeMyNotificationItem } from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { Notices } from './Notices';

function dummyProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

describe('F55: Notices time', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let viewItems = [] as ReturnType<typeof makeMyNotificationItem>[];

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    viewItems = [
      // 17:05Z on 12 Oct is 00:05 on 13 Oct in Bangkok
      makeMyNotificationItem({ id: 'n1', seq: 1, locale: 'th', title: 'Notice 1', body: 'B1', read: true, createdAt: '2026-10-12T17:05:09Z' }),
    ];
    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      if (u.pathname === '/api/v3/views/my_notifications') {
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
    setLang('th');
  });

  async function flushPromises() {
    await new Promise((r) => setTimeout(r, 10));
  }

  it('F55-NO1 th: the time reads 13/10/2026 00:05 (Bangkok day, CE, no seconds)', async () => {
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();
    expect(container.querySelector('.admin-notices-time')?.textContent?.trim()).toBe('13/10/2026 00:05');
  });

  it('F55-NO2 en: the same text', async () => {
    setLang('en');
    dispose = render(() => <Notices {...dummyProps()} />, container);
    await flushPromises();
    expect(container.querySelector('.admin-notices-time')?.textContent?.trim()).toBe('13/10/2026 00:05');
  });
});
