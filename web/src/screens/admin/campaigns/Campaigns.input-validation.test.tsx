// #4915 (input-validation audit T16, design `docs/design/web-input-validation.md`): admin › Campaigns checks
// every typed value at its field before it sends, and every refusal shows at the field it names.
// Written before the implementation by the sub-controller (claude/studio-claude-4). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// - FEE_WAIVER `fixed` / `amount` values are THB with at most 2 decimals on the server
//   (`api/crates/xc-marketing/src/domain/benefit/kinds/fee_waiver.rs:28-41`), so the editor checks them
//   with the shared `MONEY` bound (`web/src/shared/ui/bounded.ts`), not `DECIMAL_8_RE`.
// - Create and Save check, before sending: end after start, and the budget against
//   `checkDecimal(text, numeric(20, 2, 'nonNegative'))` (`campaign_create.rs:80-113`,
//   `campaign_version_save.rs:133-166`).
// - A server refusal goes through `fieldError` (`web/src/shared/i18n/field-error.ts`) with the screen's
//   field labels; a refusal on field `rules` shows the server's own sentence at the rules section
//   (`campaign_version_save.rs:303-309`).
//
// DOM hooks this test assumes (the implementer follows them): a refusal of field `<f>` is an element
// `[data-field-error="<f>"]` holding the worded line, for `endsAt` and `budgetThb` on the create form and
// on the campaign page. A refusal of a rule's calculation stays `[data-field-error="rules.calculation"]`
// inside that rule's `[data-rule-row="<n>"]` (as `Campaigns.rules.test.tsx`). A server refusal on field
// `rules` is `[data-field-error="rules"]` inside `[data-rules-editor]`.

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
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

interface SentRequest {
  method: string;
  path: string;
  body: unknown;
}

const RULES_IN_USE = 'กฎของแคมเปญนี้ถูกใช้กับคูปองที่ออกแล้ว จึงแก้กฎไม่ได้';

function dummyProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const LIST_ITEM = makeCampaignListItem({
  id: 'cmp-1',
  name: 'Golden October',
  versionId: 'ver-1',
  version: 1,
  status: 'DRAFT',
  startsAt: '2026-10-01T00:00:00Z',
  endsAt: '2026-10-31T23:59:59Z',
  budgetThb: '100000.00',
  budgetUsedThb: '0.00',
  budgetLeftThb: '100000.00',
  uses: 0,
  maxUses: 100,
  period: makeCampaignPeriodTotals({ redemptions: 0, priceCostThb: '0.00', rewardExpenseThb: '0.00' }),
});

function draftDetail() {
  return makeCampaignData({
    campaign: makeCampaignDetail({
      id: 'cmp-1',
      name: 'Golden October',
      currentVersionId: 'ver-1',
      versions: [
        makeCampaignVersionDetail({
          id: 'ver-1',
          version: 1,
          status: 'DRAFT',
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
              requiresCoupon: false,
              eligibility: {},
              calculation: { delta: '0.05' },
              caps: {},
            }),
          ],
          simulations: [],
          approval: null,
        }),
      ],
    }),
  });
}

describe('Admin Campaigns input validation (#4915, T16)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sent: SentRequest[] = [];
  let commandStatus = 200;
  let commandError: { code: string; field?: string; message: string } | null = null;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sent = [];
    commandStatus = 200;
    commandError = null;

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
      sent.push({ method, path: u.pathname, body });

      if (u.pathname === '/api/v3/views/campaigns') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: makeCampaignsData({ campaigns: [LIST_ITEM] }) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      if (u.pathname === '/api/v3/views/campaign') {
        return new Response(JSON.stringify({ version: 1, changed: true, data: draftDetail() }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      if (commandStatus !== 200 && commandError) {
        return new Response(JSON.stringify({ error: commandError }), {
          status: commandStatus,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({ id: 'cmp-new', version: 1, status: 'DRAFT' }), {
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

  // ── helpers ──────────────────────────────────────────────────────────────

  function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function field<T extends HTMLInputElement | HTMLSelectElement>(root: ParentNode, name: string): T {
    const el = root.querySelector<T>(`[name="${name}"]`);
    expect(el, `[name="${name}"]`).not.toBeNull();
    return el!;
  }

  function button(text: string): HTMLButtonElement {
    const b = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
      (x) => (x.textContent ?? '').trim() === text,
    );
    expect(b, `a button ${JSON.stringify(text)}`).toBeDefined();
    return b!;
  }

  function fieldErrorText(root: ParentNode, name: string): string {
    const el = root.querySelector(`[data-field-error="${name}"]`);
    expect(el, `a refusal shown at [data-field-error="${name}"]`).not.toBeNull();
    const text = (el!.textContent ?? '').trim();
    expect(text, `[data-field-error="${name}"] holds a worded line`).not.toBe('');
    return text;
  }

  const creates = () => sent.filter((r) => r.method === 'POST' && r.path === '/api/campaigns');
  const saves = () => sent.filter((r) => r.method === 'PUT' && r.path === '/api/campaigns/cmp-1/draft');

  async function openCreate(): Promise<void> {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    button(table.th.create).click();
    await settle();
    setValue(field(container, 'name'), 'New Campaign');
    setValue(field(container, 'startsAt'), '2026-11-01');
    setValue(field(container, 'endsAt'), '2026-11-30');
    setValue(field(container, 'budgetThb'), '50000.00');
    setValue(field(container, 'branches'), 'MAIN');
    await settle();
  }

  async function submitCreate(): Promise<void> {
    button(table.th.create).click();
    await settle();
  }

  async function openDraft(): Promise<void> {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    const row = container.querySelector<HTMLElement>('[data-campaign-id="cmp-1"]');
    expect(row, 'campaign cmp-1 is listed').not.toBeNull();
    row!.click();
    await settle();
  }

  async function saveDraft(): Promise<void> {
    button(table.th.save).click();
    await settle();
  }

  function ruleRow(n: number): HTMLElement {
    const el = container.querySelector<HTMLElement>(`[data-rules-editor] [data-rule-row="${n}"]`);
    expect(el, `rule row ${n}`).not.toBeNull();
    return el!;
  }

  async function feeWaiverFixed(value: string): Promise<HTMLElement> {
    const r = ruleRow(0);
    setValue(field<HTMLSelectElement>(r, 'benefitType'), 'FEE_WAIVER');
    await settle();
    setValue(field<HTMLSelectElement>(r, 'calcMode'), 'fixed');
    setValue(field<HTMLInputElement>(r, 'calcValue'), value);
    await settle();
    return r;
  }

  // ── §3 rows ──────────────────────────────────────────────────────────────

  it('CG1: FEE_WAIVER fixed 10.123 (3 decimals on a 2-decimal THB amount) is refused at that rule; nothing is sent', async () => {
    await openDraft();
    const r = await feeWaiverFixed('10.123');
    await saveDraft();
    expect(saves(), 'no save is sent').toEqual([]);
    fieldErrorText(r, 'rules.calculation');
  });

  it('CG2: FEE_WAIVER fixed 10.12 is accepted and sent unchanged', async () => {
    await openDraft();
    const r = await feeWaiverFixed('10.12');
    await saveDraft();
    expect(r.querySelector('[data-field-error="rules.calculation"]'), 'no refusal at the rule').toBeNull();
    expect(saves().length, 'one save').toBe(1);
    const rules = (saves()[0]!.body as { rules?: Array<Record<string, unknown>> }).rules;
    expect(rules?.[0]).toMatchObject({ benefitType: 'FEE_WAIVER', calculation: { fixed: '10.12' } });
  });

  it('CG3: create with the end date before the start date is refused at the end date; nothing is sent', async () => {
    await openCreate();
    setValue(field(container, 'startsAt'), '2026-11-30');
    setValue(field(container, 'endsAt'), '2026-11-01');
    await settle();
    await submitCreate();
    expect(creates(), 'no create is sent').toEqual([]);
    fieldErrorText(container, 'endsAt');
  });

  it('CG4: budget 1.234 is refused at the budget, on create and on the campaign page; nothing is sent', async () => {
    await openCreate();
    setValue(field(container, 'budgetThb'), '1.234');
    await settle();
    await submitCreate();
    expect(creates(), 'no create is sent').toEqual([]);
    fieldErrorText(container, 'budgetThb');

    dispose?.();
    dispose = null;
    container.textContent = '';
    await openDraft();
    setValue(field(container, 'budgetThb'), '1.234');
    await settle();
    await saveDraft();
    expect(saves(), 'no save is sent').toEqual([]);
    fieldErrorText(container, 'budgetThb');
  });

  it("CG5: a server 400 validationFailed on field 'rules' shows the server's sentence at the rules section", async () => {
    await openDraft();
    commandStatus = 400;
    commandError = { code: 'validationFailed', field: 'rules', message: RULES_IN_USE };
    await saveDraft();
    expect(saves().length, 'the save was sent').toBe(1);
    const editor = container.querySelector<HTMLElement>('[data-rules-editor]');
    expect(editor, 'the campaign page has a rules editor').not.toBeNull();
    expect(fieldErrorText(editor!, 'rules')).toContain(RULES_IN_USE);
    expect(container.textContent ?? '', 'the generic line replaces nothing').not.toContain(
      table.th['error.validationFailed'],
    );
  });

  it("CG6: a server 400 validationFailed on field 'budgetThb' shows a line naming the budget, at the budget", async () => {
    await openDraft();
    commandStatus = 400;
    commandError = { code: 'validationFailed', field: 'budgetThb', message: 'ข้อมูลไม่ถูกต้อง' };
    await saveDraft();
    expect(saves().length, 'the save was sent').toBe(1);
    expect(fieldErrorText(container, 'budgetThb')).toContain(table.th.fieldBudgetThb);
  });
});
