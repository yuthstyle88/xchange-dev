// V2-shell (#460): implementer extra tests for decisions 3 and 6
// - Decision 3: after add branch succeeds, the shell switches to the newly created branch
// - Decision 6: document.title follows v2 (admin-html:158) from the shown menu's heading

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createEffect, onCleanup, type JSX } from 'solid-js';
import type { ApiClient, ViewAnswer, ViewParams } from '../../shared/api/client';
import type { Me } from '../../shared/api/generated/common';
import { AdminShell } from './AdminShell';
import type { AdminMenu, AdminMenuProps } from './menus';

interface Branch {
  id: string;
  name: string;
}

interface Probe {
  mounts: number;
  branch: string[];
}

const BRANCHES: Branch[] = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
];

let probes: Record<string, Probe> = {};
let branches: Branch[] = [];
let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function makeMenu(id: string, order: number, title: string): AdminMenu {
  probes[id] = { mounts: 0, branch: [] };
  const Component = (props: AdminMenuProps): JSX.Element => {
    const p = probes[id]!;
    p.mounts++;
    createEffect(() => p.branch.push(props.branch()));
    onCleanup(() => {});
    return <div>{title}</div>;
  };
  return {
    id,
    order,
    label: () => title,
    icon: '▦',
    group: 'desk',
    roles: 'all',
    heading: {
      eyebrow: () => 'EYEBROW',
      title: () => title,
      subtitle: () => 'SUBTITLE',
    },
    dated: true,
    Component,
  };
}

const fakeApi = {
  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') {
      const me: Me = {
        kind: 'owner',
        id: 'owner',
        role: 'OWNER',
        branch: q.branch || 'MAIN',
        name: 'Owner',
        username: 'owner',
        branchName: q.branch || 'MAIN',
        branches: [],
      };
      return { version: 1, changed: true, data: me as T };
    }
    if (name === 'admin_branches') {
      return {
        version: 1,
        changed: true,
        data: { branches: branches.map((b) => ({ ...b })) } as T,
      };
    }
    throw new Error(`unhandled view ${name}`);
  },
  async post<B, R>(_path: string, body: B): Promise<R> {
    const b = body as { action: string; payload: Record<string, unknown> };
    if (b.action === 'adminCreateBranch') {
      const name = String(b.payload['name']);
      const newId = `B-${name}`;
      branches.push({ id: newId, name });
      return { branchId: newId, branches, ready: true } as R;
    }
    throw new Error(`unhandled post ${b.action}`);
  },
};

const api = fakeApi as unknown as ApiClient;

beforeEach(() => {
  vi.useFakeTimers();
  probes = {};
  branches = BRANCHES.map((b) => ({ ...b }));
  window.history.replaceState(null, '', '/ui/admin/#overview');
});

afterEach(async () => {
  for (const b of Array.from(document.querySelectorAll<HTMLButtonElement>('button[aria-label="ปิดหน้าต่าง"]'))) {
    if (!b.disabled) b.click();
  }
  await settle();
  dispose?.();
  dispose = undefined;
  container?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('V2-shell: extra implementer tests (#460 decisions 3 and 6)', () => {
  it('switches to the newly created branch after add branch succeeds (Controller Decision 3, admin-html:114)', async () => {
    const menus = [
      makeMenu('overview', 1, 'ภาพรวมหน้าร้าน'),
      makeMenu('transactions', 2, 'รายการแลกเงิน'),
    ];
    container = document.createElement('div');
    document.body.appendChild(container);
    dispose = render(() => <AdminShell api={api} menus={menus} />, container);
    await settle();

    const picker = container.querySelector('select')!;
    expect(picker.value).toBe('MAIN');

    // Open add branch dialog
    const addBtn = Array.from(container.querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').trim() === '＋ สาขา',
    )!;
    addBtn.click();
    await settle();

    // Type new branch name and submit
    const input = document.body.querySelector<HTMLInputElement>('input[placeholder="เช่น XC-C"]')!;
    input.value = 'XC-NEW';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = Array.from(document.body.querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').trim() === 'สร้างสาขา',
    )!;
    submitBtn.click();
    await settle();

    // Decision 3: the shell switches to the newly created branch
    expect(picker.value).toBe('B-XC-NEW');
    expect(probes['overview']!.branch).toContain('B-XC-NEW');
  });

  it('sets document.title from the active menu heading and updates on hash change (Controller Decision 6, admin-html:158)', async () => {
    const menus = [
      makeMenu('overview', 1, 'ภาพรวมหน้าร้าน'),
      makeMenu('transactions', 2, 'รายการแลกเงิน'),
    ];
    container = document.createElement('div');
    document.body.appendChild(container);
    dispose = render(() => <AdminShell api={api} menus={menus} />, container);
    await settle();

    // Initial title from overview menu
    expect(document.title).toBe('XCHANGE · ภาพรวมหน้าร้าน');

    // Navigate to transactions
    window.location.hash = '#transactions';
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await settle();

    expect(document.title).toBe('XCHANGE · รายการแลกเงิน');
  });
});
