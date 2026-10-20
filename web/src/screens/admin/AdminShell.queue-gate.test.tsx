// #3900: AdminShell mounts QueueBar only when me.permissions contains QueueOperate
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

let currentMe = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', branchName: 'MAIN', branches: [] });

const fakeApi = {
  async view<T>(name: string, _q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') return { version: 3, changed: true, data: currentMe as T };
    if (name === 'admin_branches') {
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

describe('AdminShell QueueBar permission gate (#3900)', () => {
  let queueBoardRequests: string[] = [];

  beforeEach(() => {
    queueBoardRequests = [];
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
          queueBoardRequests.push(url);
          return jsonResponse(queueCardUnchanged());
        }
        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    container?.remove();
    await settle();
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
    vi.unstubAllGlobals();
  });

  it('mounts QueueBar and requests queue_board when me.permissions contains QueueOperate', async () => {
    currentMe = {
      ...meOf({ id: 'manager', role: 'BRANCH_MANAGER', branch: 'MAIN', name: 'Manager', branchName: 'MAIN', branches: [] }),
      permissions: ['QueueOperate', 'OverviewRead'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    expect(document.querySelector('.queue-dialog')).not.toBeNull();
    expect(queueBoardRequests.length).toBeGreaterThan(0);
    expect(queueBoardRequests[0]).toContain(QUEUE_CARD_PATH);
  });

  it('does not mount QueueBar and never requests queue_board when me.permissions lacks QueueOperate', async () => {
    currentMe = {
      ...meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', branchName: 'MAIN', branches: [] }),
      permissions: ['OverviewRead'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    expect(document.querySelector('.queue-dialog')).toBeNull();
    expect(queueBoardRequests).toHaveLength(0);
  });
});
