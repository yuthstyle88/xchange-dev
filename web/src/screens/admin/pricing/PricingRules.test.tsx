// The Owner's pricing rules card (owner-set-values.md §2, xchange#6555): loading, a failed load and its retry,
// "not set" with what it means, read-only without `pricingRulesSave`, a save of the whole card as one version (an
// empty box is `null`), a refusal naming its field, and the history list.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import {
  makePricingRulesDto,
  makePricingRulesVersionDto,
  makePricingRulesViewData,
  type PricingRulesViewData,
} from '../../../shared/testing/fixtures/pricing_rules';
import { en, th } from './i18n';
import { ROUTE_PRICING_RULES } from './pricing-labels';
import { PricingRules } from './PricingRules';

type Answer = { status: number; body: unknown };

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

const SAVED = makePricingRulesVersionDto({
  version: 'v2',
  setBy: 'u-owner',
  setByName: 'owner',
  createdAt: '2026-11-11T09:00:00Z',
  rules: makePricingRulesDto({
    farFromAnchorLimitPct: '3',
    rateDecimals: [
      { fromPrice: '0', decimals: 4 },
      { fromPrice: '10', decimals: 2 },
    ],
    betterPctRange: { min: '-2', max: '2' },
  }),
});
const OLDER = makePricingRulesVersionDto({ ...SAVED, version: 'v1', createdAt: '2026-11-10T09:00:00Z' });
const WITH_HISTORY = makePricingRulesViewData({ current: SAVED, history: [SAVED, OLDER] });

/** Serves `views` in turn (an `Answer` is served as is), and `command` for the POST. */
function mount(
  views: (PricingRulesViewData | Answer)[],
  opts: { mayEdit?: boolean; command?: (body: unknown) => Answer } = {},
) {
  const calls: { method: string; path: string; body: unknown }[] = [];
  let n = 0;
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });
    let a: Answer = { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    if (method === 'GET' && path === '/api/v3/views/pricing_rules') {
      const v = views[Math.min(n++, views.length - 1)]!;
      a = 'status' in v ? v : { status: 200, body: { version: n, changed: true, data: v } };
    } else if (method === 'POST' && path === ROUTE_PRICING_RULES) {
      a = opts.command?.(body) ?? { status: 200, body: SAVED };
    }
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
  };
  const api = new ApiClient(fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <PricingRules api={api} branch="B01" mayEdit={opts.mayEdit ?? true} />, host);
  const posts = () => calls.filter((c) => c.method === 'POST');
  const gets = () => calls.filter((c) => c.method === 'GET');
  return { posts, gets };
}

const q = <T extends Element>(sel: string) => host!.querySelector<T>(sel);
const input = (name: string) => q<HTMLInputElement>(`input[name="${name}"]`)!;
function type(name: string, value: string) {
  const el = input(name);
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}
const click = (name: string) => q<HTMLButtonElement>(`button[name="${name}"]`)!.click();

describe('PricingRules', () => {
  it('shows loading, then every rule as not set with what that means when nothing was saved', async () => {
    mount([makePricingRulesViewData()]);
    expect(host!.textContent).toContain(th.loading);
    await flush();
    expect(q('[data-current]')!.textContent).toBe(th['rules.neverSaved']);
    expect(host!.querySelector('[data-not-set="thbCashStep"]')!.textContent).toContain(
      th['rules.notSetMeans.thbCashStep'],
    );
    expect(host!.querySelectorAll('[data-not-set]').length).toBe(9);
    expect(host!.textContent).toContain(th['rules.historyEmpty']);
  });

  it('shows a failed load and loads again on retry', async () => {
    const { gets } = mount([
      { status: 500, body: { error: { code: 'serverError', message: 'down' } } },
      WITH_HISTORY,
    ]);
    await flush();
    expect(q('[role="alert"]')).not.toBeNull();
    expect(q('[data-current]')).toBeNull();
    click('retry-rules');
    await flush();
    expect(gets().length).toBe(2);
    expect(q('[role="alert"]')).toBeNull();
    expect(input('farFromAnchorLimitPct').value).toBe('3');
  });

  it('shows the saved values, who saved them, and the history newest first', async () => {
    mount([WITH_HISTORY]);
    await flush();
    expect(input('farFromAnchorLimitPct').value).toBe('3');
    expect(input('betterPctRange.min').value).toBe('-2');
    expect(input('rateDecimals.1.fromPrice').value).toBe('10');
    expect(q('[data-not-set="farFromAnchorLimitPct"]')).toBeNull();
    expect(q('[data-current]')!.textContent).toContain('owner');
    const items = Array.from(host!.querySelectorAll('li[data-version]')).map((li) => li.getAttribute('data-version'));
    expect(items).toEqual(['v2', 'v1']);
  });

  it('is read-only without pricingRulesSave: no save, no band buttons, inputs read-only', async () => {
    const { posts } = mount([WITH_HISTORY], { mayEdit: false });
    await flush();
    expect(q('button[name="save-rules"]')).toBeNull();
    expect(q('button[name="band-add"]')).toBeNull();
    expect(input('farFromAnchorLimitPct').readOnly).toBe(true);
    expect(host!.textContent).toContain(th['rules.readOnly']);
    expect(posts()).toHaveLength(0);
  });

  it('saves the whole card as one version, an empty box as null, then reloads', async () => {
    const { posts, gets } = mount([WITH_HISTORY]);
    await flush();
    type('farFromAnchorLimitPct', '');
    type('thbCashStep', '0.25');
    type('branchBpCap', '300');
    click('save-rules');
    await flush();
    expect(posts()).toHaveLength(1);
    expect(posts()[0]!.body).toEqual({
      farFromAnchorLimitPct: null,
      largeMovePct: null,
      thbCashStep: '0.25',
      branchBpCap: 300,
      marketStaleAfterMinutes: null,
      betterPctRange: { min: '-2', max: '2' },
      minSpreadPctRange: null,
      advice: null,
      rateDecimals: [
        { fromPrice: '0', decimals: 4 },
        { fromPrice: '10', decimals: 2 },
      ],
    });
    expect(gets()).toHaveLength(2);
  });

  it('adds a first band from price 0 and removes a band', async () => {
    const { posts } = mount([makePricingRulesViewData()]);
    await flush();
    click('band-add');
    await flush();
    expect(input('rateDecimals.0.fromPrice').value).toBe('0');
    click('band-remove');
    await flush();
    expect(q('input[name="rateDecimals.0.fromPrice"]')).toBeNull();
    click('band-add');
    await flush();
    type('rateDecimals.0.decimals', '4');
    click('save-rules');
    await flush();
    expect((posts()[0]!.body as { rateDecimals: unknown }).rateDecimals).toEqual([{ fromPrice: '0', decimals: 4 }]);
  });

  it('shows a refusal naming its field and keeps what was typed', async () => {
    mount([WITH_HISTORY], {
      command: () => ({
        status: 400,
        body: { error: { code: 'validationFailed', field: 'betterPctRange', message: 'min > max' } },
      }),
    });
    await flush();
    type('betterPctRange.min', '5');
    click('save-rules');
    await flush();
    expect(q('[role="alert"]')!.textContent).toContain(th['rules.betterPctRange']);
    expect(input('betterPctRange.min').value).toBe('5');
    expect(input('betterPctRange.min').getAttribute('aria-invalid')).toBe('true');
  });

  it('speaks English', async () => {
    setLang('en');
    mount([makePricingRulesViewData()]);
    await flush();
    expect(host!.textContent).toContain(en['rules.title']);
    expect(host!.textContent).toContain(en['rules.notSetMeans.advice']);
  });
});
