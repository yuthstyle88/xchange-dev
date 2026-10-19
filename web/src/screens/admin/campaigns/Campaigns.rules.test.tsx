// M02 campaign rules (survey xchange#564 issuecomment-6262359159 row 23): the campaign page edits the
// benefit rules of a DRAFT version (type, calculation, eligibility, caps, requiresCoupon) and sends them
// with `campaignVersionSave` as `rules`. Written before the implementation by the sub-controller
// (claude/dell-claude-3). Do not edit, delete or weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/marketing-campaigns.md:89-96` (rules), `:292` (`campaignVersionSave`:
// "dates, branches, budget, caps, stack, priority, rules"). The wire is
// `api/src/presentation/http/v3/commands/campaign_version_save.rs`: `rules: Option<Vec<CampaignRuleInput>>`
// (`:68-70`, `#[ts(skip)]`, so the generated `CampaignVersionSaveInput` lacks it; the screen declares a local
// type), each `{ benefitType, eligibility, calculation, caps, requiresCoupon }` (`:39-50`).
// - benefit types: `RATE_IMPROVEMENT` (calculation `delta` > 0, or `percent` in (0, 1)) and `FEE_WAIVER`
//   (`percent` in (0, 1], `amount` > 0 or `fixed` > 0), values as strings
//   (`api/crates/xc-marketing/src/domain/benefit/kinds/*.rs`); the server answers `validationFailed` with
//   field `rules.benefitType` / `rules.calculation.*` (`:254-268`);
// - eligibility keys (`domain/benefit/eligibility.rs`): `branches`, `channels`, `currencies`, `shopSide`
//   (BUY/SELL), `minThb`, `tiers`, `segment`, `referred`, `requiresCoupon`;
// - caps (`domain/benefit/rule.rs:27-35`, `deny_unknown_fields`): `perTransactionMaxThb`, `perCustomerUses`,
//   `perCustomerPeriodDays`.
//
// DOM hooks this test assumes (the implementer follows them): the editor is `[data-rules-editor]` on the
// campaign page; each rule is `[data-rule-row="<n>"]` (0-based, in rule order) holding
// `select[name="benefitType"]`, `select[name="calcMode"]` (`delta` / `percent` for RATE_IMPROVEMENT;
// `percent` / `amount` / `fixed` for FEE_WAIVER), `input[name="calcValue"]`, `input[name="currencies"]`
// (comma-separated codes, empty = any), `select[name="shopSide"]` (`""` / `BUY` / `SELL`),
// `input[name="minThb"]`, `input[name="perTransactionMaxThb"]`, `input[name="perCustomerUses"]`,
// `input[name="perCustomerPeriodDays"]`, `input[name="requiresCoupon"]` (checkbox), and a button whose text
// is the i18n key `removeRule`. One button `addRule` adds a rule. A refusal on screen is
// `[data-field-error="rules.calculation"]` in the rule row. The page's existing save button (`save`) sends
// the version. Eligibility keys the editor does not show (`segment`, `tiers`, `branches`, `channels`,
// `referred`) are kept as stored.
// Keys of `./i18n.ts` (th and en): `rulesTitle`, `addRule`, `removeRule`, `benefitType`, `calcMode`,
// `calcValue`, `currencies`, `shopSide`, `minThb`, `perTransactionMaxThb`, `perCustomerUses`,
// `perCustomerPeriodDays`, `requiresCoupon`, `calcInvalid`.

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeCampaignData,
  makeCampaignDetail,
  makeCampaignListItem,
  makeCampaignPeriodTotals,
  makeCampaignRuleDetail,
  makeCampaignVersionDetail,
  makeCampaignsData,
  type CampaignData,
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
  headers: Record<string, string>;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const SAMPLE_LIST_ITEM = makeCampaignListItem({
  id: 'cmp-1',
  name: 'Golden October',
  versionId: 'ver-1',
  version: 1,
  status: 'ACTIVE',
  startsAt: '2026-10-01T00:00:00Z',
  endsAt: '2026-10-31T23:59:59Z',
  budgetThb: '100000.00',
  budgetUsedThb: '25000.50',
  budgetLeftThb: '74999.50',
  uses: 42,
  maxUses: 100,
  period: makeCampaignPeriodTotals({
    redemptions: 15,
    priceCostThb: '12345.67',
    rewardExpenseThb: '3000.00',
  }),
});

describe('M02 campaign rules editor (row 23)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  let listStatus = 200;
  let listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
  let listErrorCode: string | undefined = undefined;

  let detailStatus = 200;
  let detailData = makeCampaignData({ campaign: null });
  let detailErrorCode: string | undefined = undefined;

  let commandStatus = 200;
  let commandOutput: unknown = {};
  let commandErrorCode: string | undefined = undefined;

  async function openCampaign(campaignId = 'cmp-1'): Promise<void> {
    const itemEl = container.querySelector(`[data-campaign-id="${campaignId}"]`);
    expect(itemEl, `campaign ${campaignId} link/row exists on list`).not.toBeNull();
    (itemEl as HTMLElement).click();
    await settle();
  }

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    listStatus = 200;
    listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
    listErrorCode = undefined;

    detailStatus = 200;
    detailData = makeCampaignData({ campaign: null });
    detailErrorCode = undefined;

    commandStatus = 200;
    commandOutput = {};
    commandErrorCode = undefined;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body, headers });

      if (u.pathname === '/api/v3/views/campaigns') {
        if (listStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: listErrorCode ?? 'serverError', message: 'Failed to load campaigns' } }),
            { status: listStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true, data: listData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (u.pathname === '/api/v3/views/campaign') {
        if (detailStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: detailErrorCode ?? 'serverError', message: 'Failed to load campaign' } }),
            { status: detailStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true, data: detailData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Real command routes and methods from each const ROUTE / METHOD:
      // - campaignCreate: POST /api/campaigns
      // - campaignVersionSave: PUT /api/campaigns/{id}/draft
      // - campaignVersionNew: POST /api/campaigns/{id}/versions
      // - campaignSimulate: POST /api/campaigns/{id}/simulate
      // - campaignSubmit: POST /api/campaigns/{id}/submit
      // - campaignPause: POST /api/campaigns/{id}/pause
      // - campaignResume: POST /api/campaigns/{id}/resume
      // - couponIssue: POST /api/campaigns/{id}/coupons
      const isCampaignCommand =
        (method === 'POST' && u.pathname === '/api/campaigns') ||
        (method === 'PUT' && /^\/api\/campaigns\/[^/]+\/draft$/.test(u.pathname)) ||
        (method === 'POST' && /^\/api\/campaigns\/[^/]+\/(versions|simulate|submit|pause|resume|coupons)$/.test(u.pathname));

      if (isCampaignCommand) {
        if (commandStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: commandErrorCode ?? 'validationFailed', message: 'Command failed' } }),
            { status: commandStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify(commandOutput), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(async () => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    await settle();
  });


  // ── this file's fixtures and helpers ──────────────────────────────────────────

  const RULE_KEYS = [
    'rulesTitle',
    'addRule',
    'removeRule',
    'benefitType',
    'calcMode',
    'calcValue',
    'currencies',
    'shopSide',
    'minThb',
    'perTransactionMaxThb',
    'perCustomerUses',
    'perCustomerPeriodDays',
    'requiresCoupon',
    'calcInvalid',
  ] as const;

  function draftWithRules(status = 'DRAFT') {
    return makeCampaignData({
      campaign: makeCampaignDetail({
        id: 'cmp-1',
        name: 'Golden October',
        currentVersionId: 'ver-1',
        versions: [
          makeCampaignVersionDetail({
            id: 'ver-1',
            version: 1,
            status,
            startsAt: '2026-10-01T00:00:00Z',
            endsAt: '2026-10-31T23:59:59Z',
            budgetThb: '100000.00',
            maxUses: 100,
            stackGroup: 'DEFAULT',
            priority: 10,
            branches: ['MAIN'],
            rules: [
              makeCampaignRuleDetail({
                id: 'rule-1',
                ruleNo: 1,
                benefitType: 'RATE_IMPROVEMENT',
                benefitClass: 'PRICE',
                requiresCoupon: true,
                eligibility: { currencies: ['USD'], shopSide: 'SELL', minThb: '1000', segment: true, tiers: ['GOLD'] },
                calculation: { delta: '0.05' },
                caps: { perTransactionMaxThb: '500', perCustomerUses: 3 },
              }),
            ],
            simulations: [],
            approval: null,
          }),
        ],
      }),
    });
  }

  function editor(): HTMLElement {
    const el = container.querySelector<HTMLElement>('[data-rules-editor]');
    expect(el, 'the campaign page has a rules editor [data-rules-editor]').not.toBeNull();
    return el!;
  }

  function ruleRow(n: number): HTMLElement {
    const el = editor().querySelector<HTMLElement>(`[data-rule-row="${n}"]`);
    expect(el, `rule row ${n}`).not.toBeNull();
    return el!;
  }

  function ctl<T extends HTMLInputElement | HTMLSelectElement>(row: ParentNode, name: string): T {
    const el = row.querySelector<T>(`[name="${name}"]`);
    expect(el, `[name="${name}"]`).not.toBeNull();
    return el!;
  }

  function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function say(key: (typeof RULE_KEYS)[number], lang: 'th' | 'en' = 'th'): string {
    const v = (table as unknown as Record<string, Record<string, string>>)[lang]?.[key];
    expect(v, `campaigns i18n ${lang}.${key}`).toBeTruthy();
    return v!;
  }

  function buttonText(name: string): HTMLButtonElement {
    const b = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
      (x) => (x.textContent ?? '').trim() === name,
    );
    expect(b, `a button ${JSON.stringify(name)}`).toBeDefined();
    return b!;
  }

  function puts(): SentRequest[] {
    return sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/campaigns/cmp-1/draft');
  }

  async function saveVersion(): Promise<void> {
    buttonText(table.th.save).click();
    await settle();
  }

  async function openDraft(data: CampaignData = draftWithRules()): Promise<void> {
    detailData = data;
    commandOutput = { id: 'ver-1', version: 1, status: 'DRAFT' };
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');
  }

  describe('M02 rules: the editor shows the stored rules (marketing-campaigns.md:89-96)', () => {
    it("a DRAFT version shows its rule: type, calculation, eligibility, caps and coupon", async () => {
      await openDraft();
      const r = ruleRow(0);
      expect(ctl<HTMLSelectElement>(r, 'benefitType').value).toBe('RATE_IMPROVEMENT');
      expect(ctl<HTMLSelectElement>(r, 'calcMode').value).toBe('delta');
      expect(ctl<HTMLInputElement>(r, 'calcValue').value).toBe('0.05');
      expect(ctl<HTMLInputElement>(r, 'currencies').value.replace(/\s/g, '')).toBe('USD');
      expect(ctl<HTMLSelectElement>(r, 'shopSide').value).toBe('SELL');
      expect(ctl<HTMLInputElement>(r, 'minThb').value).toBe('1000');
      expect(ctl<HTMLInputElement>(r, 'perTransactionMaxThb').value).toBe('500');
      expect(ctl<HTMLInputElement>(r, 'perCustomerUses').value).toBe('3');
      expect(ctl<HTMLInputElement>(r, 'perCustomerPeriodDays').value).toBe('');
      expect(ctl<HTMLInputElement>(r, 'requiresCoupon').checked).toBe(true);
    });

    it('a version that is not DRAFT shows its rules read-only', async () => {
      await openDraft(draftWithRules('ACTIVE'));
      const r = ruleRow(0);
      for (const name of ['benefitType', 'calcMode', 'calcValue', 'minThb', 'requiresCoupon']) {
        expect(ctl<HTMLInputElement>(r, name).disabled, `${name} is read-only`).toBe(true);
      }
      expect([...editor().querySelectorAll('button')].some((b) => (b.textContent ?? '').trim() === say('addRule') && !b.disabled)).toBe(false);
    });
  });

  describe('M02 rules: save sends rules (campaign_version_save.rs:39-70)', () => {
    it('save sends the edited rule, keeping the stored eligibility keys the editor does not show', async () => {
      await openDraft();
      const r = ruleRow(0);
      setValue(ctl<HTMLInputElement>(r, 'calcValue'), '0.08');
      setValue(ctl<HTMLInputElement>(r, 'perCustomerPeriodDays'), '30');
      await settle();
      await saveVersion();
      expect(puts().length, 'one save').toBe(1);
      const body = puts()[0]!.body as Record<string, unknown>;
      expect(body.rules).toEqual([
        {
          benefitType: 'RATE_IMPROVEMENT',
          calculation: { delta: '0.08' },
          eligibility: { currencies: ['USD'], shopSide: 'SELL', minThb: '1000', segment: true, tiers: ['GOLD'] },
          caps: { perTransactionMaxThb: '500', perCustomerUses: 3, perCustomerPeriodDays: 30 },
          requiresCoupon: true,
        },
      ]);
    });

    it('an added FEE_WAIVER rule goes second; empty fields are left out', async () => {
      await openDraft();
      buttonText(say('addRule')).click();
      await settle();
      const r = ruleRow(1);
      setValue(ctl<HTMLSelectElement>(r, 'benefitType'), 'FEE_WAIVER');
      await settle();
      setValue(ctl<HTMLSelectElement>(r, 'calcMode'), 'percent');
      setValue(ctl<HTMLInputElement>(r, 'calcValue'), '0.5');
      await settle();
      await saveVersion();
      const rules = (puts()[0]!.body as Record<string, unknown>).rules as Array<Record<string, unknown>>;
      expect(rules.length).toBe(2);
      expect(rules[1]).toEqual({
        benefitType: 'FEE_WAIVER',
        calculation: { percent: '0.5' },
        eligibility: {},
        caps: {},
        requiresCoupon: false,
      });
    });

    it('removing the only rule sends an empty list', async () => {
      await openDraft();
      const remove = [...ruleRow(0).querySelectorAll<HTMLButtonElement>('button')].find(
        (b) => (b.textContent ?? '').trim() === say('removeRule'),
      );
      expect(remove, 'the rule has a remove button').toBeDefined();
      remove!.click();
      await settle();
      await saveVersion();
      expect((puts()[0]!.body as Record<string, unknown>).rules).toEqual([]);
    });

    it('a calculation outside its range is refused on screen and nothing is sent', async () => {
      await openDraft();
      const r = ruleRow(0);
      for (const [mode, bad] of [
        ['delta', '0'],
        ['delta', '-1'],
        ['percent', '1'],
        ['percent', 'x'],
      ] as const) {
        setValue(ctl<HTMLSelectElement>(r, 'calcMode'), mode);
        setValue(ctl<HTMLInputElement>(r, 'calcValue'), bad);
        await settle();
        await saveVersion();
        expect(puts(), `nothing sent for ${mode} ${bad}`).toEqual([]);
        const err = r.querySelector('[data-field-error="rules.calculation"]');
        expect(err, `refused: ${mode} ${bad}`).not.toBeNull();
        expect((err!.textContent ?? '').trim()).toBe(say('calcInvalid'));
      }
    });

    it("the server's refusal (rules already used by coupons) is shown, not swallowed", async () => {
      await openDraft();
      commandStatus = 400;
      commandErrorCode = 'validationFailed';
      await saveVersion();
      expect(puts().length).toBe(1);
      expect(container.textContent ?? '').toMatch(/Command failed|ไม่ถูกต้อง|validation/i);
    });
  });

  describe('M02 rules: Thai and English (W10)', () => {
    it('every key this test reads is in both tables, the English one with no Thai', () => {
      for (const k of RULE_KEYS) {
        expect(say(k, 'th')).toMatch(/[฀-๿]/);
        expect(say(k, 'en')).not.toMatch(/[฀-๿]/);
      }
    });
  });
});
