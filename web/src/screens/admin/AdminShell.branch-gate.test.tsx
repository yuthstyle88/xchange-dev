// #4045: AdminShell reads admin_branches only when me.permissions contains CompatRead
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

describe('AdminShell admin_branches CompatRead permission gate (#4045)', () => {
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
    dispose?.();
    dispose = undefined;
    container?.remove();
    await settle();
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
    vi.unstubAllGlobals();
  });

  // Row 1: Marketing permissions without CompatRead -> no admin_branches read, shell remains online
  it('does not read admin_branches and stays online when me.permissions lacks CompatRead (row 1)', async () => {
    currentMe = {
      ...meOf({ id: 'marketing', role: 'MARKETING', branch: 'MAIN', name: 'Marketing', branchName: 'MAIN', branches: [] }),
      permissions: ['MarketingCampaigns', 'MarketingMembers', 'MarketingPerformance', 'AttendanceClock', 'SignedIn'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    expect(branchReads).toHaveLength(0);
    expect(document.querySelector('.notice-bar')?.textContent).not.toContain('ยังไม่พร้อมใช้งาน');
  });

  // Row 2: User with CompatRead -> admin_branches read on mount
  it('reads admin_branches when me.permissions includes CompatRead (row 2)', async () => {
    currentMe = {
      ...meOf({ id: 'admin', role: 'ADMIN', branch: 'MAIN', name: 'Admin', branchName: 'MAIN', branches: [] }),
      permissions: ['CompatRead', 'OverviewRead'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    expect(branchReads).toHaveLength(1);
    expect(branchReads[0]).toEqual({ branch: 'MAIN' });
  });

  // Row 3: Marketing user (Row 1), then Refresh -> still no admin_branches read
  it('still sends no admin_branches read when Refresh is clicked without CompatRead (row 3)', async () => {
    currentMe = {
      ...meOf({ id: 'marketing', role: 'MARKETING', branch: 'MAIN', name: 'Marketing', branchName: 'MAIN', branches: [] }),
      permissions: ['MarketingCampaigns', 'MarketingMembers', 'MarketingPerformance', 'AttendanceClock', 'SignedIn'],
    };
    sharedSession().updateMe(currentMe);

    await mount();
    expect(branchReads).toHaveLength(0);

    const refreshBtn = Array.from(document.querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').trim() === 'รีเฟรช' || (b.textContent ?? '').trim() === 'Refresh',
    );
    expect(refreshBtn, 'refresh button exists in shell').toBeDefined();
    refreshBtn!.click();
    await settle();

    expect(branchReads).toHaveLength(0);
    expect(document.querySelector('.notice-bar')?.textContent).not.toContain('ยังไม่พร้อมใช้งาน');
  });

  // Row 4: Marketing user (Row 1) -> heading shows branch id fallback
  it('shows branch id fallback in menu heading when branches are empty without CompatRead (row 4)', async () => {
    currentMe = {
      ...meOf({ id: 'marketing', role: 'MARKETING', branch: 'MAIN', name: 'Marketing', branchName: 'MAIN', branches: [] }),
      branch: 'MAIN',
      permissions: ['MarketingCampaigns', 'MarketingMembers', 'MarketingPerformance', 'AttendanceClock', 'SignedIn'],
    };
    sharedSession().updateMe(currentMe);

    await mount();

    const subtitle = document.querySelector('.menu-heading .subtitle')?.textContent;
    expect(subtitle).toContain(' · MAIN');
  });
});
