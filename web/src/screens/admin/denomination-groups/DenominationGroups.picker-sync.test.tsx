// #2548: Admin denomination groups — currency <select> must stay synced with selectedCurrency()
// across re-reads (save/remove/add or branch switch where currency still exists) and reset to ''
// when switching to a branch whose currencies do not include selectedCurrency().
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

interface RecordedWrite {
  path: string;
  body: Record<string, unknown>;
}

describe('Admin denomination groups currency picker sync (#2548 §8 & §9)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let viewReplies: unknown[];
  let writes: RecordedWrite[];

  const branchAInitial = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'EUR',
        groups: [{ group: 'EUR_STD', instrumentKey: 'EUR_1', removable: false }],
      },
      {
        currency: 'USD',
        groups: [{ group: 'USD_HIGH', instrumentKey: 'USD_100', removable: true }],
      },
    ],
  };

  const branchAAfterSave = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'EUR',
        groups: [{ group: 'EUR_STD', instrumentKey: 'EUR_1', removable: false }],
      },
      {
        currency: 'USD',
        groups: [{ group: 'USD_HIGH', instrumentKey: 'USD_100_NEW', removable: true }],
      },
    ],
  };

  const branchBWithoutUsd = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'EUR',
        groups: [{ group: 'EUR_B', instrumentKey: 'EUR_B1', removable: false }],
      },
      {
        currency: 'GBP',
        groups: [{ group: 'GBP_B', instrumentKey: 'GBP_B1', removable: true }],
      },
    ],
  };

  const branchBWithUsdSecond = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'EUR',
        groups: [{ group: 'EUR_B', instrumentKey: 'EUR_B1', removable: false }],
      },
      {
        currency: 'USD',
        groups: [{ group: 'USD_B_GROUP', instrumentKey: 'USD_B100', removable: true }],
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
    viewReplies = [];
    writes = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/admin_denomination_groups' && method === 'GET') {
          const next = viewReplies.shift() ?? branchAInitial;
          return jsonResponse(200, { version: 1, changed: true, data: next });
        }

        if (
          (path === '/api/denomination-groups' ||
            path === '/api/denomination-groups/map' ||
            path === '/api/denomination-groups/remove') &&
          method === 'POST'
        ) {
          const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {};
          writes.push({ path, body });
          return jsonResponse(200, { ok: true });
        }

        return jsonResponse(200, { version: 1, changed: false });
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

  function getSelect(): HTMLSelectElement {
    const select = root.querySelector<HTMLSelectElement>('.denomination-groups-currency-picker select');
    expect(select).not.toBeNull();
    return select!;
  }

  function selectCurrency(currency: string): void {
    const select = getSelect();
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

  function getAddButton(): HTMLButtonElement | undefined {
    return [...root.querySelectorAll<HTMLButtonElement>('.add-group-form button')].find(
      (b) => b.textContent?.trim() === screenTh.add,
    );
  }

  it("row 1: Pick USD (second option), save a key (re-read) -> Picker still shows USD; list is USD's", async () => {
    viewReplies = [branchAInitial, branchAAfterSave];
    mount('BRANCH_A');
    await settle();

    // Pick USD (the second currency option after EUR)
    selectCurrency('USD');
    await settle();
    expect(getSelect().value).toBe('USD');
    expect(root.querySelector('[data-group="USD_HIGH"]')).not.toBeNull();
    expect(root.querySelector('[data-group="EUR_STD"]')).toBeNull();

    // Edit the board key on USD_HIGH and click Save -> triggers POST /api/denomination-groups/map + re-read
    const row = root.querySelector('[data-group="USD_HIGH"]')!;
    const keyInput = row.querySelector<HTMLInputElement>('input')!;
    keyInput.value = 'USD_100_NEW';
    keyInput.dispatchEvent(new Event('input', { bubbles: true }));
    keyInput.dispatchEvent(new Event('change', { bubbles: true }));

    const saveBtn = [...row.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === screenTh.save,
    )!;
    saveBtn.click();
    await settle();

    expect(writes).toHaveLength(1);
    expect(writes[0]).toEqual({
      path: '/api/denomination-groups/map',
      body: {
        currency: 'USD',
        group: 'USD_HIGH',
        provider: 'RATE_BOARD',
        instrumentKey: 'USD_100_NEW',
        branch: 'BRANCH_A',
      },
    });

    // After the re-read, the <select> DOM element must still show 'USD' (not jump to first option 'EUR')
    expect(getSelect().value).toBe('USD');
    expect(root.querySelector('[data-group="USD_HIGH"] .board-key-display')?.textContent).toBe(
      'USD_100_NEW',
    );
    expect(root.querySelector('[data-group="EUR_STD"]')).toBeNull();
  });

  it('row 2: Branch switch to a branch without USD -> Picker shows the placeholder; Add sends nothing until a currency is picked', async () => {
    viewReplies = [branchAInitial, branchBWithoutUsd, branchBWithoutUsd];
    const { setBranch } = mount('BRANCH_A');
    await settle();

    selectCurrency('USD');
    await settle();
    expect(getSelect().value).toBe('USD');

    // Type a group name and keep a reference to the Add button before switching branches
    typeNewGroup('NEW_GROUP_1');
    const staleAddButtonRef = getAddButton()!;
    expect(staleAddButtonRef).toBeDefined();

    // Switch to BRANCH_B which only has EUR and GBP (no USD)
    setBranch('BRANCH_B');
    await settle();

    // Picker must show the placeholder ('') and the add form is hidden
    expect(getSelect().value).toBe('');
    expect(getAddButton()).toBeUndefined();
    expect(root.querySelectorAll('[data-group]')).toHaveLength(0);

    // Clicking the stale Add button reference (before a currency is picked on B) sends nothing
    staleAddButtonRef.click();
    await settle();
    expect(writes).toEqual([]);

    // Once a valid currency on BRANCH_B ('GBP') is picked, Add sends for GBP on BRANCH_B
    selectCurrency('GBP');
    await settle();
    expect(getSelect().value).toBe('GBP');
    typeNewGroup('NEW_GROUP_1');
    getAddButton()!.click();
    await settle();

    expect(writes).toEqual([
      {
        path: '/api/denomination-groups',
        body: {
          currency: 'GBP',
          group: 'NEW_GROUP_1',
          branch: 'BRANCH_B',
        },
      },
    ]);
  });

  it('row 3: Branch switch to a branch that also has USD (second option) -> USD stays selected in the picker and list', async () => {
    viewReplies = [branchAInitial, branchBWithUsdSecond];
    const { setBranch } = mount('BRANCH_A');
    await settle();

    selectCurrency('USD');
    await settle();
    expect(getSelect().value).toBe('USD');

    // Switch to BRANCH_B which also has USD as its second option
    setBranch('BRANCH_B');
    await settle();

    expect(getSelect().value).toBe('USD');
    expect(root.querySelector('[data-group="USD_B_GROUP"]')).not.toBeNull();
    expect(root.querySelector('[data-group="EUR_B"]')).toBeNull();
  });

  // Kept row from §8:
  // - "Group save/remove/add flows" is kept and pinned by DenominationGroups.test.tsx and DenominationGroups.write-branch.test.tsx.
});
