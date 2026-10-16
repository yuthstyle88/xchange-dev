import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

interface DeferredRequest {
  branch: string;
  path: string;
  resolve: (body: unknown, status?: number) => void;
  rejectNetwork: () => void;
}

describe('Admin denomination groups write branch-switch guard (#2304)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredRequest[];
  let pendingWrites: DeferredRequest[];

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
    pendingWrites = [];
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
              path,
              resolve: (body: unknown, status = 200) => resolve(jsonResponse(status, body)),
              rejectNetwork: () => reject(new TypeError('Failed to fetch')),
            });
          });
        }

        if (
          (path === '/api/denomination-groups' ||
            path === '/api/denomination-groups/map' ||
            path === '/api/denomination-groups/remove') &&
          method === 'POST'
        ) {
          const parsed = init?.body ? (JSON.parse(String(init.body)) as { branch?: string }) : {};
          const branch = parsed.branch ?? '';
          return new Promise<Response>((resolve, reject) => {
            pendingWrites.push({
              branch,
              path,
              resolve: (body: unknown, status = 200) => resolve(jsonResponse(status, body)),
              rejectNetwork: () => reject(new TypeError('Failed to fetch')),
            });
          });
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

  function typeNewGroup(code: string): void {
    const addInput = root.querySelector<HTMLInputElement>('.add-group-form input')!;
    expect(addInput).not.toBeNull();
    addInput.value = code;
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addInput.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function getAddButton(): HTMLButtonElement {
    const btn = [...root.querySelectorAll<HTMLButtonElement>('.add-group-form button')].find(
      (b) => b.textContent?.trim() === screenTh.add,
    );
    expect(btn).toBeDefined();
    return btn!;
  }

  it('Add on A fails after a switch to B: no toast and no add-form error on B', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_A_GROUP');
    getAddButton().click();
    await settle();
    expect(pendingWrites.length).toBe(1);
    expect(pendingWrites[0]!.branch).toBe('BRANCH_A');

    // Switch to Branch B before A's add answers
    setBranch('BRANCH_B');
    await settle();
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    // A's in-flight add fails after switching to B
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Add failed on A' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
  });

  it('Remove or map on A fails after a switch to B: no toast on B', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({
      version: 1,
      changed: true,
      data: {
        provider: 'RATE_BOARD',
        currencies: [
          {
            currency: 'USD',
            groups: [
              { group: 'BRANCH_A_GROUP', instrumentKey: 'USD_A', removable: true },
              { group: 'BRANCH_A_SECOND', instrumentKey: 'USD_A2', removable: true },
            ],
          },
        ],
      },
    });
    await settle();

    selectCurrency('USD');
    await settle();

    const row1 = root.querySelector('[data-group="BRANCH_A_GROUP"]')!;
    const row2 = root.querySelector('[data-group="BRANCH_A_SECOND"]')!;
    expect(row1).not.toBeNull();
    expect(row2).not.toBeNull();
    const saveBtn = [...row1.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === screenTh.save,
    )!;
    const removeBtn = [...row2.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === screenTh.remove,
    )!;

    // Trigger both map and remove on Branch A
    saveBtn.click();
    removeBtn.click();
    await settle();
    expect(pendingWrites.length).toBe(2);
    expect(pendingWrites[0]!.path).toBe('/api/denomination-groups/map');
    expect(pendingWrites[1]!.path).toBe('/api/denomination-groups/remove');

    // Switch to Branch B before A's writes settle
    setBranch('BRANCH_B');
    await settle();
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    // Both of A's writes fail after the switch to B
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Map failed on A' } },
      500,
    );
    pendingWrites[1]!.resolve(
      { error: { code: 'serverError', message: 'Remove failed on A' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector('.row-error')).toBeNull();
  });

  it("A's add in flight, switch to B: B's Add button is enabled", async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_A_GROUP');
    getAddButton().click();
    await settle();
    expect(pendingWrites.length).toBe(1);
    expect(getAddButton().disabled).toBe(true);

    // Switch to Branch B while A's add is still in flight
    setBranch('BRANCH_B');
    await settle();
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    const addInputB = root.querySelector<HTMLInputElement>('.add-group-form input')!;
    expect(getAddButton().disabled).toBe(false);
    expect(addInputB.disabled).toBe(false);
  });

  it("Switch to B while B's read is in flight: A's groups are not shown and A's failing write does not leak to B", async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).not.toBeNull();

    typeNewGroup('NEW_A_GROUP');
    getAddButton().click();
    await settle();
    expect(pendingWrites.length).toBe(1);

    // Switch to Branch B; B's read (pendingReads[1]) is now in flight
    setBranch('BRANCH_B');
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH_B');

    // While B's read is in flight, A's groups must not be shown
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
    expect(root.querySelectorAll('[data-group]').length).toBe(0);

    // A's write fails while B's read is still in flight
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Add failed on A' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
  });

  it('Add on A fails with no switch: worded error is shown on A and cleared upon switching to B', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_A_GROUP');
    getAddButton().click();
    await settle();
    expect(pendingWrites.length).toBe(1);

    // Add on A fails with no branch switch
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Add failed on A' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent).toContain(sharedTh['error.serverError']);
    expect(root.querySelector('.add-group-form .error-msg')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(getAddButton().disabled).toBe(false);

    // Switching to Branch B clears A's add-form error
    setBranch('BRANCH_B');
    await settle();
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
  });

  it('Edge case — write on A succeeds after switch to B: B Add stays enabled, B input is preserved, and re-read reads B', async () => {
    const { setBranch } = mount('BRANCH_A');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: branchAData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_A_GROUP');
    getAddButton().click();
    await settle();
    expect(pendingWrites.length).toBe(1);
    expect(pendingWrites[0]!.branch).toBe('BRANCH_A');

    // Switch to Branch B while A's add is still in flight
    setBranch('BRANCH_B');
    await settle();
    pendingReads[1]!.resolve({ version: 2, changed: true, data: branchBData });
    await settle();

    expect(getAddButton().disabled).toBe(false);
    typeNewGroup('DRAFT_ON_B');

    // A's write succeeds after switching to B
    pendingWrites[0]!.resolve({ ok: true }, 200);
    await settle();

    expect(pendingReads.length).toBe(3);
    expect(pendingReads[2]!.branch).toBe('BRANCH_B');
    expect(root.querySelector<HTMLInputElement>('.add-group-form input')?.value).toBe('DRAFT_ON_B');

    pendingReads[2]!.resolve({ version: 3, changed: true, data: branchBData });
    await settle();

    expect(root.querySelector('[data-group="BRANCH_B_GROUP"]')).not.toBeNull();
    expect(root.querySelector('[data-group="BRANCH_A_GROUP"]')).toBeNull();
  });
});
