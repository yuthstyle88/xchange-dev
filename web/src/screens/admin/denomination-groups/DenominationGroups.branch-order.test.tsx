import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

interface DeferredRead {
  branch: string;
  resolve: (body: unknown, status?: number) => void;
  rejectNetwork: () => void;
}

describe('Admin denomination groups branch-order guard (#2291)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredRead[];

  const branchAData = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'USD',
        groups: [{ group: 'BRANCH_A_GROUP', instrumentKey: 'USD_A', removable: true }],
      },
    ],
  };

  const branchBData = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'USD',
        groups: [{ group: 'BRANCH_B_GROUP', instrumentKey: 'USD_B', removable: true }],
      },
    ],
  };

  function jsonResponse(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  beforeEach(() => {
    setLang('th');
    toast('');
    pendingReads = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/admin_denomination_groups' && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? '';
          return new Promise<Response>((resolve, reject) => {
            pendingReads.push({
              branch,
              resolve: (body: unknown, status = 200) => resolve(jsonResponse(status, body)),
              rejectNetwork: () => reject(new TypeError('Failed to fetch')),
            });
          });
        }

        if (path === '/api/denomination-groups' && method === 'POST') {
          return Promise.resolve(jsonResponse(200, { ok: true }));
        }

        return Promise.resolve(jsonResponse(200, { version: 1, changed: false }));
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    toast('');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
    await settle();
  });

  function mount(initialBranch = 'BRANCH_A'): { setBranch: (b: string) => void } {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <DenominationGroups branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    return { setBranch };
  }

  function selectCurrency(currency: string): void {
    const select = root.querySelector<HTMLSelectElement>('select')!;
    expect(select).not.toBeNull();
    select.value = currency;
    select.dispatchEvent(new Event('input', { bubbles: true }));
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('Branch A to B, A answers last: B groups are shown and A stale answer is ignored', async () => {
    const { setBranch } = mount('BRANCH_A');
    expect(pendingReads.length).toBe(1);
    expect(pendingReads[0]!.branch).toBe('BRANCH_A');

    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH_B');

    // Branch B answers first
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    selectCurrency('USD');
    await settle();
    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();

    // Branch A answers last (stale)
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
  });

  it('An older read fails after a newer success: no toast and B view stays', async () => {
    const { setBranch } = mount('BRANCH_A');
    expect(pendingReads.length).toBe(1);

    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(2);

    // Newer read (Branch B) succeeds first
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    selectCurrency('USD');
    await settle();
    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();

    // Older read (Branch A) fails afterwards
    pendingReads[0]!.resolve(
      { error: { code: 'serverError', message: 'Stale failure' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();
  });

  it('B answers with no data: shows empty view, not A groups', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).not.toBeNull();

    // Switch to Branch B, which answers with no data
    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(2);

    pendingReads[1]!.resolve({ version: 2, changed: false });
    await settle();

    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
    expect(root.querySelectorAll('[data-group]').length).toBe(0);
  });

  it('A read fails: view is empty and shows worded error toast even if an older read resolves later', async () => {
    const { setBranch } = mount('BRANCH_A');
    expect(pendingReads.length).toBe(1);

    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(2);

    // Current branch read fails
    pendingReads[1]!.resolve(
      { error: { code: 'serverError', message: 'Internal error' } },
      500,
    );
    await settle();

    const toastEl = root.querySelector('.toast');
    expect(toastEl).not.toBeNull();
    expect(toastEl!.textContent).toContain(sharedTh['error.serverError']);

    // Stale read from Branch A resolves afterwards; view must stay empty
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    const currencyOptions = [...root.querySelectorAll('select option')].filter(
      (o) => (o as HTMLOptionElement).value !== '',
    );
    expect(currencyOptions.length).toBe(0);
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
  });

  it('Edge case — add group succeeds, then its re-read goes through the same guard when branch switches', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).not.toBeNull();

    // Add group on Branch A -> triggers POST then loadView() (pendingReads[1])
    const addInput = root.querySelector<HTMLInputElement>('.add-group-form input')!;
    addInput.value = 'NEW_A_GROUP';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addInput.dispatchEvent(new Event('change', { bubbles: true }));

    const addBtn = [...root.querySelectorAll<HTMLButtonElement>('.add-group-form button')].find(
      (b) => b.textContent?.trim() === screenTh.add,
    )!;
    addBtn.click();
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH_A');

    // Switch to Branch B before the post-add re-read finishes (pendingReads[2])
    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(3);
    expect(pendingReads[2]!.branch).toBe('BRANCH_B');

    // Branch B resolves first
    pendingReads[2]!.resolve({ version: 3, changed: true, data: branchBData });
    await settle();

    // Then the stale post-add re-read for Branch A resolves
    pendingReads[1]!.resolve({
      version: 2,
      changed: true,
      data: {
        provider: 'RATE_BOARD',
        currencies: [
          {
            currency: 'USD',
            groups: [
              { group: 'BRANCH_A_GROUP', instrumentKey: 'USD_A', removable: true },
              { group: 'NEW_A_GROUP', instrumentKey: null, removable: true },
            ],
          },
        ],
      },
    });
    await settle();

    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
    expect(root.querySelector('[data-group="NEW_A_GROUP"]')).toBeNull();
  });
});
