// Pricing formula FE T6 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 F1–F11, §3a DOM hooks),
// written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add
// tests in a file of your own. The shared parts are replaced by `parts/formula-stubs.testing.tsx` so this ticket does
// not wait for T1–T5. Contracts: view `pricing_formula` (views/pricing_formula.rs), `pricingFormulaSave`
// POST /api/pricing/formula (commands/pricing_formula_save.rs:52-140).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeOrganisationFormulaDto, makePricingFormulaData, type PricingFormulaData } from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { previewCalls, resetFormulaStubs } from '../parts/formula-stubs.testing';
import { toast } from '../parts/toast';
import { PricingFormula } from './PricingFormula';

vi.mock('../parts/FormulaEditor', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/FormulaPreviewTable', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/FormulaRefusal', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/formula-preview', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/spread-failure', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/toast', () => ({ toast: vi.fn(), Toast: () => null }));

const VIEW = '/api/v3/views/pricing_formula';
const SAVE = '/api/pricing/formula';
const DEFAULT = {
  buyFormula: 'ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)',
  sellFormula: 'ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)',
};

function data(over: Partial<PricingFormulaData> = {}) {
  return makePricingFormulaData({
    organisation: makeOrganisationFormulaDto({
      id: 'f-3',
      version: 3,
      buyFormula: 'ref_mid - 0.05',
      sellFormula: 'ref_mid + 0.05',
      validFrom: '2026-10-29T10:00:00Z',
      approvedBy: 'exec-1',
      reason: 'narrow',
    }),
    default: DEFAULT,
    inputs: ['ref_buy', 'ref_sell', 'ref_mid', 'buy_pct', 'sell_pct', 'tier_buy_pct', 'tier_sell_pct'].map((name) => ({
      name,
    })),
    branches: [
      { branch: 'MAIN', source: 'ORGANISATION', own: null },
      { branch: 'B2', source: 'BRANCH', own: { buyFormula: 'ref_buy * 0.99', sellFormula: 'ref_sell * 1.01' } },
      { branch: 'B3', source: 'DEFAULT', own: null },
    ],
    ...over,
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

describe('pricing-formula-web T6: the organisation formula screen', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let viewAnswer: () => Response;
  let saveAnswer: () => Response;
  let reads: string[];
  let saves: Record<string, unknown>[];
  const [branch, setBranch] = createSignal('MAIN');

  beforeEach(() => {
    setLang('th');
    resetFormulaStubs();
    vi.mocked(toast).mockClear();
    setBranch('MAIN');
    reads = [];
    saves = [];
    viewAnswer = () => json({ version: 1, changed: true, data: data() });
    saveAnswer = () => json({ id: 'f-4', version: 4, notChecked: [] });
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.startsWith(VIEW)) {
          reads.push(url);
          return viewAnswer();
        }
        if (url.startsWith(SAVE) && !url.startsWith(`${SAVE}/preview`)) {
          saves.push(JSON.parse(String(init?.body ?? '{}')));
          return saveAnswer();
        }
        return json({ error: { code: 'notFound', message: url } }, 404);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  function mount(): void {
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <PricingFormula {...props} />, host);
  }

  const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
  const text = (sel: string) => (q(sel)?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buyField = () => q<HTMLTextAreaElement>('textarea[data-field="buyFormula"]');
  const sellField = () => q<HTMLTextAreaElement>('textarea[data-field="sellFormula"]');
  const reasonField = () => q<HTMLTextAreaElement>('textarea[data-field="reason"]');
  const saveButton = () => q<HTMLButtonElement>('button[data-action="formula-save"]');

  function type(el: HTMLTextAreaElement | null, value: string): void {
    expect(el).not.toBeNull();
    el!.value = value;
    el!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  async function loaded(): Promise<void> {
    await vi.waitFor(() => expect(buyField()).not.toBeNull());
  }

  it('F1: the organisation formula is in the editor and the header names its version, approver and time (ICT)', async () => {
    mount();
    await loaded();
    expect(reads[0]).toContain('branch=MAIN');
    expect(buyField()!.value).toBe('ref_mid - 0.05');
    expect(sellField()!.value).toBe('ref_mid + 0.05');
    const header = text('[data-testid="formula-current"]');
    expect(header).toContain('3');
    expect(header).toContain('exec-1');
    expect(header).toContain('17:00');
  });

  it('F2: no organisation formula ⇒ the default texts, and the header says the system default applies', async () => {
    viewAnswer = () => json({ version: 1, changed: true, data: data({ organisation: null }) });
    mount();
    await loaded();
    expect(buyField()!.value).toBe(DEFAULT.buyFormula);
    expect(sellField()!.value).toBe(DEFAULT.sellFormula);
    expect(text('[data-testid="formula-current"]')).toContain('ยังไม่มีสูตรขององค์กร');
  });

  it('F3: every branch with its source in Thai, and a branch formula shows its texts', async () => {
    mount();
    await loaded();
    expect(text('tr[data-branch="MAIN"]')).toContain('สูตรขององค์กร');
    expect(text('tr[data-branch="B2"]')).toContain('สูตรของสาขา');
    expect(text('tr[data-branch="B2"]')).toContain('ref_buy * 0.99');
    expect(text('tr[data-branch="B2"]')).toContain('ref_sell * 1.01');
    expect(text('tr[data-branch="B3"]')).toContain('สูตรมาตรฐานของระบบ');
  });

  it('F4: the live preview runs as the organisation formula for this branch, on the editor texts', async () => {
    mount();
    await loaded();
    const o = previewCalls.at(-1)!;
    expect(o.scope).toBe('ORGANISATION');
    expect(o.branch()).toBe('MAIN');
    type(buyField(), 'ref_mid - 0.04');
    expect(o.buy()).toBe('ref_mid - 0.04');
    expect(o.sell()).toBe('ref_mid + 0.05');
    await vi.waitFor(() => expect(o.enabled()).toBe(true));
  });

  it('F5: a blank reason or a blank text sends nothing and marks the field', async () => {
    mount();
    await loaded();
    saveButton()!.click();
    await vi.waitFor(() => expect(q('[data-field-error="reason"]')).not.toBeNull());
    type(reasonField(), 'narrow the spread');
    type(sellField(), '   ');
    saveButton()!.click();
    await vi.waitFor(() => expect(q('[data-field-error="sellFormula"]')).not.toBeNull());
    expect(saves).toHaveLength(0);
  });

  it('F6: a save sends the trimmed texts and reason, toasts, lists notChecked, reads again and clears the reason', async () => {
    saveAnswer = () => json({ id: 'f-4', version: 4, notChecked: [{ branch: 'B3', currency: 'KRW' }] });
    mount();
    await loaded();
    type(buyField(), '  ref_mid - 0.04 ');
    type(reasonField(), ' narrow the spread ');
    saveButton()!.click();
    await vi.waitFor(() => expect(saves).toHaveLength(1));
    expect(saves[0]).toEqual({
      branch: 'MAIN',
      buyFormula: 'ref_mid - 0.04',
      sellFormula: 'ref_mid + 0.05',
      reason: 'narrow the spread',
    });
    await vi.waitFor(() => expect(vi.mocked(toast)).toHaveBeenCalled());
    await vi.waitFor(() => expect(text('[data-testid="formula-not-checked"]')).toContain('B3 KRW'));
    await vi.waitFor(() => expect(reads.length).toBe(2));
    expect(reasonField()!.value).toBe('');
  });

  it('F7: a refused check lists every failing line, and nothing else changes', async () => {
    saveAnswer = () =>
      json(
        {
          error: {
            code: 'validationFailed',
            field: 'buyFormula',
            message: 'MAIN USD buy 33.88 > sell 33.72\nB3 JPY tier 4 FOLLOW buy 0.3 > sell 0.2',
          },
        },
        400,
      );
    mount();
    await loaded();
    type(buyField(), 'ref_sell * 1.01');
    type(reasonField(), 'try');
    saveButton()!.click();
    await vi.waitFor(() => expect(host.querySelectorAll('[data-stub="refusal"] li')).toHaveLength(2));
    expect(text('[data-stub="refusal"]')).toContain('MAIN USD buy 33.88 > sell 33.72');
    expect(text('[data-stub="refusal"]')).toContain('B3 JPY tier 4 FOLLOW');
    expect(buyField()!.value).toBe('ref_sell * 1.01');
    expect(reasonField()!.value).toBe('try');
    expect(reads).toHaveLength(1);
  });

  it('F8: a parse refusal on the sell text shows under the sell field, no refusal list', async () => {
    saveAnswer = () =>
      json({ error: { code: 'validationFailed', field: 'sellFormula', message: 'unknown input `foo`' } }, 400);
    mount();
    await loaded();
    type(sellField(), 'foo + 1');
    type(reasonField(), 'try');
    saveButton()!.click();
    await vi.waitFor(() => expect(q('[data-field-error="sellFormula"]')).not.toBeNull());
    expect(q('[data-stub="refusal"]')).toBeNull();
  });

  it('F9: a role refusal is worded and the texts stay', async () => {
    saveAnswer = () => json({ error: { code: 'roleNotAllowedHere', message: 'EXECUTIVE only (test)' } }, 403);
    mount();
    await loaded();
    type(buyField(), 'ref_mid - 0.04');
    type(reasonField(), 'try');
    saveButton()!.click();
    await vi.waitFor(() => expect(q('[data-field-error="save"]')).not.toBeNull());
    expect(text('[data-field-error="save"]')).toMatch(/บทบาทนี้ไม่มีสิทธิ์|EXECUTIVE only \(test\)/);
    expect(buyField()!.value).toBe('ref_mid - 0.04');
  });

  it('F10: a failed read is worded with a retry that reads again', async () => {
    viewAnswer = () => json({ error: { code: 'serverError', message: 'boom' } }, 500);
    mount();
    await vi.waitFor(() => expect(q('[data-testid="formula-load-error"]')).not.toBeNull());
    expect(buyField()).toBeNull();
    viewAnswer = () => json({ version: 1, changed: true, data: data() });
    q<HTMLButtonElement>('button[data-action="formula-retry"]')!.click();
    await loaded();
    expect(reads).toHaveLength(2);
  });

  it('F11: a branch change reads the view again for that branch', async () => {
    mount();
    await loaded();
    setBranch('B2');
    await vi.waitFor(() => expect(reads.at(-1)).toContain('branch=B2'));
  });
});
