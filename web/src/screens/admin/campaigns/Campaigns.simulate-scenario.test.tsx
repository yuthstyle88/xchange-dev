// Written before the implementation by the test writer claude/e8020026 (sub-agent; XC FE lane controller). Do not edit, delete or weaken.
// #4842: admin campaigns Simulate sends a scenario (design `docs/design/web-campaign-simulate.md` SIM-1..SIM-6).
// Root cause: `handleSimulate` always sent `{campaignId, scenarios: []}`, which the server refuses since #1989.

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeCampaignData,
  makeCampaignDetail,
  makeCampaignListItem,
  makeCampaignSimulateOutput,
  makeCampaignVersionDetail,
  makeCampaignsData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
}

function dummyProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

/** A DRAFT campaign whose current version names `branches`. */
function draft(id: string, name: string, branches: string[]) {
  return makeCampaignDetail({
    id,
    name,
    currentVersionId: `${id}-v1`,
    versions: [
      makeCampaignVersionDetail({
        id: `${id}-v1`,
        version: 1,
        status: 'DRAFT',
        startsAt: '2026-10-01T00:00:00Z',
        endsAt: '2026-10-31T23:59:59Z',
        budgetThb: '100000.00',
        stackGroup: 'DEFAULT',
        priority: 10,
        branches,
        rules: [],
        simulations: [],
        approval: null,
      }),
    ],
  });
}

describe('#4842 Campaigns: Simulate sends one scenario (SIM-1..SIM-6)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sent: SentRequest[] = [];
  let details: Record<string, ReturnType<typeof draft>> = {};
  let commandStatus = 200;
  let commandError: { code: string; message: string; field?: string } | null = null;

  beforeEach(() => {
    setLang('en');
    container = document.createElement('div');
    document.body.appendChild(container);
    sent = [];
    commandStatus = 200;
    commandError = null;
    details = {
      'cmp-1': draft('cmp-1', 'Two branches', ['B1', 'B2']),
      'cmp-2': draft('cmp-2', 'One branch', ['B3']),
      'cmp-0': draft('cmp-0', 'No branch', []),
    };

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }
      sent.push({ method, path: u.pathname, search: u.search, body });
      const json = (payload: unknown, status = 200) =>
        new Response(JSON.stringify(payload), { status, headers: { 'content-type': 'application/json' } });

      if (u.pathname === '/api/v3/views/campaigns') {
        const campaigns = Object.values(details).map((d) =>
          makeCampaignListItem({ id: d.id, name: d.name, versionId: `${d.id}-v1`, version: 1, status: 'DRAFT' }),
        );
        return json({ version: 1, changed: true, data: makeCampaignsData({ campaigns }) });
      }
      if (u.pathname === '/api/v3/views/campaign') {
        const id = u.searchParams.get('id') ?? 'cmp-1';
        return json({ version: 1, changed: true, data: makeCampaignData({ campaign: details[id]! }) });
      }
      const isCommand =
        (method === 'PUT' && /^\/api\/campaigns\/[^/]+\/draft$/.test(u.pathname)) ||
        (method === 'POST' && /^\/api\/campaigns\/[^/]+\/(versions|simulate|submit|pause|resume|coupons)$/.test(u.pathname));
      if (isCommand) {
        if (commandStatus !== 200) {
          return json({ error: commandError ?? { code: 'validationFailed', message: 'Command failed' } }, commandStatus);
        }
        return json(u.pathname.endsWith('/simulate') ? makeCampaignSimulateOutput() : {});
      }
      return json({ version: 1, changed: false });
    });
  });

  afterEach(async () => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    await settle();
  });

  async function mountAndOpen(campaignId: string): Promise<void> {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign(campaignId);
  }

  async function openCampaign(campaignId: string): Promise<void> {
    const row = container.querySelector(`[data-campaign-id="${campaignId}"]`);
    expect(row, `campaign ${campaignId} row exists on the list`).not.toBeNull();
    (row as HTMLElement).click();
    await settle();
  }

  async function back(): Promise<void> {
    const btn = Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-header button')).find(
      (b) => b.textContent?.trim() === table.en.back,
    );
    expect(btn, 'Back button exists').toBeDefined();
    btn!.click();
    await settle();
  }

  function byTestId(id: string): HTMLInputElement | HTMLSelectElement | null {
    return container.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-testid="${id}"]`);
  }

  function valueOf(id: string): string | null {
    return byTestId(id)?.value ?? null;
  }

  /** Types into a scenario field. A missing field is left alone, so the assertion after it says what is wrong. */
  function setField(id: string, value: string): void {
    const el = byTestId(id);
    if (!el) return;
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function actionButton(key: 'simulate' | 'save'): HTMLButtonElement {
    const btn = Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
      (b) => b.textContent?.trim() === table.en[key],
    );
    expect(btn, `${key} button exists in the detail actions`).toBeDefined();
    return btn!;
  }

  function simulateRequests(campaignId: string): SentRequest[] {
    return sent.filter((r) => r.method === 'POST' && r.path === `/api/campaigns/${campaignId}/simulate`);
  }

  function detailReads(campaignId: string): number {
    return sent.filter(
      (r) => r.path === '/api/v3/views/campaign' && new URLSearchParams(r.search).get('id') === campaignId,
    ).length;
  }

  it('(a) SIM-1/SIM-2: a version naming [B1, B2] shows the scenario row: branch select B1|B2 (B1 preselected), currency USD, amount 100', async () => {
    await mountAndOpen('cmp-1');

    const row = container.querySelector('.xc-campaign-sim-row');
    expect(row, 'SIM-1: the scenario row .xc-campaign-sim-row is shown').not.toBeNull();

    const branch = byTestId('sim-branch');
    const currency = byTestId('sim-currency');
    const amount = byTestId('sim-amount');
    expect(branch?.tagName, 'SIM-1: sim-branch is a <select>').toBe('SELECT');
    expect(currency?.tagName, 'SIM-1: sim-currency is an <input>').toBe('INPUT');
    expect(amount?.tagName, 'SIM-1: sim-amount is an <input>').toBe('INPUT');
    for (const el of [branch!, currency!, amount!]) {
      expect(row!.contains(el), `${el.dataset.testid} is inside the scenario row`).toBe(true);
      expect(el.closest('.xc-campaign-sim-field'), `${el.dataset.testid} sits in a .xc-campaign-sim-field`).not.toBeNull();
    }

    const options = Array.from((branch as HTMLSelectElement).options).map((o) => o.value);
    expect(options, 'SIM-2: the options are the version branches only').toEqual(['B1', 'B2']);
    expect(branch!.value, 'SIM-2: the first branch is preselected').toBe('B1');
    expect(currency!.value, 'SIM-1: currency defaults to USD').toBe('USD');
    expect(amount!.value, 'SIM-1: amount defaults to 100').toBe('100');

    const text = row!.textContent ?? '';
    for (const key of ['simBranch', 'simCurrency', 'simAmount'] as const) {
      expect(text, `the row is labelled with ${key}`).toContain(table.en[key]);
    }
  });

  it('(b) SIM-3: Simulate is enabled with the defaults, disabled for an amount of empty, 0 or -5 or a currency of US or usd1, and a click then sends nothing', async () => {
    await mountAndOpen('cmp-1');
    expect(actionButton('simulate').disabled, 'with a branch and the defaults, Simulate is enabled').toBe(false);

    for (const bad of ['', '0', '-5']) {
      setField('sim-amount', bad);
      await settle();
      expect(actionButton('simulate').disabled, `SIM-3: Simulate is disabled for the amount '${bad}'`).toBe(true);
      actionButton('simulate').click();
      await settle();
    }

    setField('sim-amount', '100');
    for (const bad of ['US', 'usd1']) {
      setField('sim-currency', bad);
      await settle();
      expect(actionButton('simulate').disabled, `SIM-3: Simulate is disabled for the currency '${bad}'`).toBe(true);
      actionButton('simulate').click();
      await settle();
    }

    expect(simulateRequests('cmp-1'), 'SIM-3: no simulate request while the scenario is invalid').toHaveLength(0);

    setField('sim-currency', 'USD');
    await settle();
    expect(actionButton('simulate').disabled, 'a valid scenario enables Simulate again').toBe(false);
  });

  it('(c) SIM-4: branch B2, currency usd and amount 100.50 send exactly one scenario {B2, USD, 100.50}', async () => {
    await mountAndOpen('cmp-1');
    setField('sim-branch', 'B2');
    setField('sim-currency', 'usd');
    setField('sim-amount', '100.50');
    await settle();

    actionButton('simulate').click();
    await settle();

    const reqs = simulateRequests('cmp-1');
    expect(reqs, 'one simulate request').toHaveLength(1);
    expect(reqs[0]!.body, 'SIM-4: the body carries exactly the one typed scenario, currency upper-cased').toEqual({
      campaignId: 'cmp-1',
      scenarios: [{ branchId: 'B2', currency: 'USD', amount: '100.50' }],
    });
  });

  it('(d) SIM-2: a version with no branches shows no select, the simNeedsBranch line, a disabled Simulate, and sends nothing', async () => {
    await mountAndOpen('cmp-0');

    const note = container.querySelector('p.xc-campaign-sim-note');
    expect(note?.textContent?.trim(), 'SIM-2: the worded simNeedsBranch line shows').toBe(table.en.simNeedsBranch);
    expect(byTestId('sim-branch'), 'SIM-2: no branch select when the version names no branch').toBeNull();
    expect(actionButton('simulate').disabled, 'SIM-2: Simulate stays disabled without a branch').toBe(true);

    actionButton('simulate').click();
    await settle();
    expect(simulateRequests('cmp-0'), 'no simulate request without a branch').toHaveLength(0);
  });

  it('(e) SIM-5: a 400 validationFailed on amount shows the existing worded detail error and keeps the row', async () => {
    commandStatus = 400;
    commandError = { code: 'validationFailed', message: 'amount has too many decimals', field: 'amount' };
    await mountAndOpen('cmp-1');

    actionButton('simulate').click();
    await settle();

    const reqs = simulateRequests('cmp-1');
    expect(reqs, 'one simulate request').toHaveLength(1);
    expect(reqs[0]!.body, 'the defaults are sent as one scenario').toEqual({
      campaignId: 'cmp-1',
      scenarios: [{ branchId: 'B1', currency: 'USD', amount: '100' }],
    });

    const error = container.querySelector('[data-field-error="amount"]')?.textContent?.trim();
    expect(error, 'SIM-5: the existing campaignError wording shows').toBe(`${table.en.simAmount} is not valid`);
    expect(valueOf('sim-amount'), 'the typed amount stays after the refusal').toBe('100');
  });

  it('(f) SIM-6: the row keeps its values across a reload of the same campaign (save, simulate) and resets to the defaults for a different campaign', async () => {
    await mountAndOpen('cmp-1');
    setField('sim-branch', 'B2');
    setField('sim-currency', 'eur');
    setField('sim-amount', '250');
    await settle();

    const readsBeforeSave = detailReads('cmp-1');
    actionButton('save').click();
    await settle();
    expect(detailReads('cmp-1'), 'Save reloads the same campaign').toBeGreaterThan(readsBeforeSave);
    expect(
      [valueOf('sim-branch'), valueOf('sim-currency'), valueOf('sim-amount')],
      'SIM-6: values kept after the reload that follows Save',
    ).toEqual(['B2', 'EUR', '250']);

    const readsBeforeSimulate = detailReads('cmp-1');
    actionButton('simulate').click();
    await settle();
    expect(detailReads('cmp-1'), 'Simulate reloads the same campaign').toBeGreaterThan(readsBeforeSimulate);
    expect(
      [valueOf('sim-branch'), valueOf('sim-currency'), valueOf('sim-amount')],
      'SIM-6: values kept after the reload that follows Simulate',
    ).toEqual(['B2', 'EUR', '250']);

    await back();
    await openCampaign('cmp-2');
    expect(
      [valueOf('sim-branch'), valueOf('sim-currency'), valueOf('sim-amount')],
      'SIM-6: a different campaign starts from its first branch and the defaults',
    ).toEqual(['B3', 'USD', '100']);
  });

  it('(g) guard: the Simulate button exists in the detail actions', async () => {
    await mountAndOpen('cmp-1');
    expect(actionButton('simulate').textContent?.trim()).toBe(table.en.simulate);
  });
});
