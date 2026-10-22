import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import {
  makeAdminBranchItem,
  makeAdminBranchesData,
} from '../../../shared/testing/fixtures';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

const B01 = 'XC-B01';

let disposers: Array<() => void> = [];
let profileReply: (() => Promise<Response>) | null = null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

beforeEach(() => {
  setLang('th');
  profileReply = null;
  document.body.innerHTML = '';
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const u = new URL(String(input), 'http://localhost');
      if (u.pathname === '/api/v3/views/admin_branches') {
        return json(200, {
          version: 1,
          changed: true,
          data: makeAdminBranchesData({
            branches: [makeAdminBranchItem({ id: B01, name: 'Silom Branch' })],
          }),
        });
      }
      if (u.pathname === '/api/v3/views/admin_branches_profile') {
        if (profileReply) return profileReply();
        return json(200, { version: 1, changed: true });
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function mount(): HTMLElement {
  const [branch] = createSignal(B01);
  const [shown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });
  const container = document.createElement('div');
  document.body.appendChild(container);
  disposers.push(
    render(() => <BranchesScreen branch={branch} period={period} shown={shown} />, container),
  );
  return container;
}

describe('R3-7 (#4744): BranchesScreen loading and empty fallback', () => {
  it('B1: the profile read is pending -> loading text shown; saving not shown', async () => {
    let resolveProfile!: (res: Response) => void;
    profileReply = () =>
      new Promise<Response>((r) => {
        resolveProfile = r;
      });

    const container = mount();
    await settle();

    const emptyText = container.querySelector('.branches-empty')?.textContent?.trim();
    expect(emptyText).toBe(table.th.loading);
    expect(container.textContent).not.toContain(table.th.saving);

    resolveProfile(json(200, { version: 1, changed: true }));
    await settle();
  });

  it('B2: the read fails -> the error alert shows; noBranches not shown', async () => {
    profileReply = async () =>
      json(500, { error: { code: 'serverError', message: 'Internal Server Error' } });

    const container = mount();
    await settle();

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent?.trim()).toBe(sharedTh['error.serverError']);
    expect(container.textContent).not.toContain(table.th.noBranches);
  });

  it('B3: the read answers no profile -> noBranches shown as today', async () => {
    profileReply = async () => json(200, { version: 1, changed: true });

    const container = mount();
    await settle();

    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector('.branches-empty')?.textContent?.trim()).toBe(
      table.th.noBranches,
    );
  });
});
