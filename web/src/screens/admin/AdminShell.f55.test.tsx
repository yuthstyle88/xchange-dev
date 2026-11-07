// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// The notice bar's "last update" is an instant with no day shown beside it, so it reads `dd/mm/yyyy HH:mm` (no seconds).
// Written before the implementation by the ticket author. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../shared/api/client';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../shared/testing/page-wide';
import { meOf } from '../../test-support/me';
import { sharedSession } from '../../shared/session/session';
import { AdminShell } from './AdminShell';
import type { AdminMenu } from './menus';

function fixtureMenu(id: string, order: number, label: string): AdminMenu {
  const Component = (): JSX.Element => <section data-fixture-menu={id}>{`body of ${id}`}</section>;
  return {
    id,
    order,
    label: () => label,
    icon: '▦',
    group: 'desk',
    needs: 'all',
    heading: { eyebrow: () => 'EYEBROW', title: () => label, subtitle: () => 'SUBTITLE' },
    dated: false,
    Component,
  };
}

const BRANCHES = [{ id: 'MAIN', name: 'สำนักงานใหญ่' }];

let currentMe = meOf({ id: 'marketing', role: 'MARKETING', branch: 'MAIN', name: 'Marketing', branchName: 'MAIN', branches: [] });
let branchReads: ViewParams[] = [];

const fakeApi = {
  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') return { version: 3, changed: true, data: currentMe as T };
    if (name === 'admin_branches') {
      branchReads.push(q);
      if (!currentMe.permissions.includes('CompatRead')) {
        throw new Error('400 roleNotAllowedHere');
      }
      return { version: 3, changed: true, data: { branches: BRANCHES.map((b) => ({ ...b })) } as T };
    }
    throw new Error(`the shell reads no view ${name}`);
  },
  async page<R>(name: string): Promise<R[]> {
    throw new Error(`the shell reads no page ${name}`);
  },
  async act<B, R>(area: string, action: string, _body: B): Promise<R> {
    throw new Error(`the shell calls no v3 action ${area}/${action}`);
  },
  async post<B, R>(_path: string, _body: B): Promise<R> {
    throw new Error('the tests post nothing');
  },
};

const api = fakeApi as unknown as ApiClient;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;
const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

async function mount(): Promise<void> {
  window.history.replaceState(null, '', '/ui/admin/#overview');
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={api} menus={[fixtureMenu('overview', 1, 'Overview')]} />, container);
  await settle();
}

describe('F55: AdminShell last-update stamp', () => {
  beforeEach(() => {
    branchReads = [];
    try {
      window.localStorage.clear();
    } catch {
      // storage blocked
    }
    sharedSession().updateMe(currentMe);
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string) => {
        const url = String(input);
        if (url.includes('/health/ready')) {
          return jsonResponse({ version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-08T00:00:00Z' });
        }
        if (url.startsWith('/api/v3/views/me')) return jsonResponse({ version: 1, changed: true, data: currentMe });
        if (url.startsWith(QUEUE_CARD_PATH)) {
          return jsonResponse(queueCardUnchanged());
        }
        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: true,
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      })),
    );
  });

  afterEach(async () => {
    vi.useRealTimers();
    dispose?.();
    dispose = undefined;
    container?.remove();
    await settle();
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
    vi.unstubAllGlobals();
  });

  it('F55-AS1: the last update reads 04/11/2026 13:23 (Bangkok, CE, no seconds)', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-11-04T06:23:45Z'));
    currentMe = {
      ...meOf({ id: 'admin', role: 'ADMIN', branch: 'MAIN', name: 'Admin', branchName: 'MAIN', branches: [] }),
      permissions: ['CompatRead', 'OverviewRead'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    const bar = document.querySelector('.notice-bar')?.textContent ?? '';
    expect(bar).toMatch(/(อัปเดตล่าสุด |Last update: )04\/11\/2026 13:23$/);
    expect(bar).not.toContain('13:23:45');
  });
});
