// #5200 (V.9 PV-d2): `BranchPolicy` edits one currency's branch policy (buy/sell bp, own, follow) and saves it with
// one keyed POST /api/pricing/branch-policy. Up-front tests, written before the implementation. Do not edit, delete or
// weaken these tests.
//
// [required] by these tests (ticket #5200 §3 and its behaviour table B1..B8):
// - Inputs `input[name="buyBp"]`, `input[name="sellBp"]`; when not MAIN also checkboxes `input[name="own"]` and
//   `input[name="follow"]`; save is `button[name="save-policy"]`.
// - "A line at an input" = an element `[data-field-error]` with non-empty text that is a sibling of that input (same
//   parentElement), as in `Pricing.input-validation.test.tsx` and `PricingPolicy.tsx`.
// - bp is a whole number with BRANCH_BP_MIN <= bp <= BRANCH_BP_MAX; anything else shows `bpRange` ({min}, {max}) at
//   that input and posts nothing.
// - Server refusals go through `fieldError` (a 400 `validationFailed` on `buyBp` shows `input.invalid` at buyBp).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import {
  makePricingBranchPolicyDto,
  makePricingRowDto,
  type PricingRowDto,
} from '../../../shared/testing/fixtures/pricing_board';
import {
  makePricingBranchPolicySaveOutput,
  type PricingBranchPolicySaveOutput,
} from '../../../shared/testing/fixtures/pricing_branch_policy_save';
import { boardFetch } from './__fixtures__/pricing-board.fixture';
import { BranchPolicy } from './BranchPolicy';
import { th as pricingTh } from './i18n';
import { BRANCH_BP_MAX, BRANCH_BP_MIN, ROUTE_PRICING_BRANCH_POLICY } from './pricing-labels';

const THAI = /[฀-๿]/;
const INVALID_SUFFIX = sharedTh['input.invalid'].replace('{field}', '').trim();
const BP_RANGE = pricingTh.bpRange
  .replace('{min}', String(BRANCH_BP_MIN))
  .replace('{max}', String(BRANCH_BP_MAX));

/** B1's row: USD, the branch's own currency row, following MAIN. */
const USD_ROW = makePricingRowDto({
  currency: 'USD',
  branchPolicy: makePricingBranchPolicyDto({ buyBp: 10, sellBp: -5, own: false, follow: true, from: 'currency' }),
});
const USD_FROM_STAR = makePricingRowDto({
  currency: 'USD',
  branchPolicy: makePricingBranchPolicyDto({ buyBp: 0, sellBp: 0, own: false, follow: false, from: '*' }),
});

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;
function unmount() {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
}
afterEach(() => {
  unmount();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(opts?: {
  row?: PricingRowDto;
  isMain?: boolean;
  mayEdit?: boolean;
  command?: (path: string, body: unknown) => { status: number; body: unknown };
}) {
  const onSaved = vi.fn<(out: PricingBranchPolicySaveOutput) => void>();
  const isMain = opts?.isMain ?? false;
  const defaultCommand = (path: string) =>
    path === ROUTE_PRICING_BRANCH_POLICY
      ? { status: 200, body: makePricingBranchPolicySaveOutput({ saved: true }) }
      : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
  const server = boardFetch(undefined, opts?.command ?? defaultCommand);
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  const el = document.createElement('div');
  host = el;
  document.body.appendChild(el);
  dispose = render(
    () => (
      <BranchPolicy
        api={api}
        branch={isMain ? 'MAIN' : 'B2'}
        row={opts?.row ?? USD_ROW}
        isMain={isMain}
        mayEdit={opts?.mayEdit ?? true}
        onSaved={onSaved}
      />
    ),
    el,
  );
  return { server, onSaved };
}

function input(name: string): HTMLInputElement {
  const el = host!.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `input[name=${name}] exists`).not.toBeNull();
  return el!;
}

function type(name: string, value: string) {
  const el = input(name);
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

async function save() {
  const b = host!.querySelector<HTMLButtonElement>('button[name="save-policy"]');
  expect(b, 'button[name=save-policy] exists').not.toBeNull();
  b!.click();
  await flush();
}

/** The worded line at an input: a `[data-field-error]` sibling (same parentElement) with text. */
function lineAt(name: string): string {
  return input(name).parentElement?.querySelector('[data-field-error]')?.textContent?.trim() ?? '';
}

const posts = (server: ReturnType<typeof boardFetch>) => server.calls.filter((c) => c.method === 'POST');

describe('BranchPolicy (#5200)', () => {
  it('B1 a currency row at branch B2 - bp inputs 10 / -5, follow checked, "ตั้งเฉพาะสกุล" shown', () => {
    mount();
    expect(input('buyBp').value).toBe('10');
    expect(input('sellBp').value).toBe('-5');
    expect(input('own').checked).toBe(false);
    expect(input('follow').checked).toBe(true);
    expect(host!.textContent).toContain('ตั้งเฉพาะสกุล');
  });

  it('B2 from "*" - "ใช้ค่ากลางของสาขา" shown', () => {
    mount({ row: USD_FROM_STAR });
    expect(host!.textContent).toContain('ใช้ค่ากลางของสาขา');
    expect(host!.textContent).not.toContain('ตั้งเฉพาะสกุล');
  });

  it('B3 not MAIN, buyBp 25, save - one keyed POST with currency, buyBp, sellBp, own, follow; onSaved called', async () => {
    const { server, onSaved } = mount();
    type('buyBp', '25');
    await save();

    const sent = posts(server);
    expect(sent).toHaveLength(1);
    expect(sent[0].path).toBe(ROUTE_PRICING_BRANCH_POLICY);
    expect(sent[0].body).toEqual({ currency: 'USD', buyBp: 25, sellBp: -5, own: false, follow: true });
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(onSaved).toHaveBeenCalledWith(makePricingBranchPolicySaveOutput({ saved: true }));
  });

  it('B4 isMain - no own/follow inputs; the body has only currency, buyBp, sellBp', async () => {
    const { server } = mount({ isMain: true });
    expect(host!.querySelector('input[name="own"]')).toBeNull();
    expect(host!.querySelector('input[name="follow"]')).toBeNull();
    await save();

    const sent = posts(server);
    expect(sent).toHaveLength(1);
    expect(sent[0].path).toBe(ROUTE_PRICING_BRANCH_POLICY);
    expect(sent[0].body).toEqual({ currency: 'USD', buyBp: 10, sellBp: -5 });
  });

  it('B5 buyBp 301, -301, 1.5 or blank - bpRange at buyBp; no POST', async () => {
    for (const bad of [String(BRANCH_BP_MAX + 1), String(BRANCH_BP_MIN - 1), '1.5', '']) {
      const { server, onSaved } = mount();
      type('buyBp', bad);
      await save();

      expect(lineAt('buyBp'), `bpRange at buyBp for "${bad}"`).toBe(BP_RANGE);
      expect(lineAt('sellBp'), `nothing at sellBp for "${bad}"`).toBe('');
      expect(posts(server), `no POST for "${bad}"`).toHaveLength(0);
      expect(onSaved).not.toHaveBeenCalled();
      unmount();
    }
  });

  it('B6 buyBp 300 and -300 - accepted (POST sent)', async () => {
    for (const edge of [BRANCH_BP_MAX, BRANCH_BP_MIN]) {
      const { server } = mount();
      type('buyBp', String(edge));
      await save();

      const sent = posts(server);
      expect(sent, `one POST for ${edge}`).toHaveLength(1);
      expect((sent[0].body as Record<string, unknown>).buyBp).toBe(edge);
      expect(lineAt('buyBp')).toBe('');
      unmount();
    }
  });

  it('B7 mayEdit false - inputs disabled; no save-policy button', () => {
    mount({ mayEdit: false });
    for (const name of ['buyBp', 'sellBp', 'own', 'follow']) {
      expect(input(name).disabled, `input[name=${name}] disabled`).toBe(true);
    }
    expect(host!.querySelector('button[name="save-policy"]')).toBeNull();
  });

  it('B8 POST answers 400 validationFailed on buyBp - field error at buyBp; onSaved not called', async () => {
    const serverText = 'buyBp out of range (raw server text)';
    const { server, onSaved } = mount({
      command: () => ({
        status: 400,
        body: { error: { code: 'validationFailed', field: 'buyBp', message: serverText } },
      }),
    });
    await save();

    expect(posts(server)).toHaveLength(1);
    const line = lineAt('buyBp');
    expect(line, 'a worded line at buyBp').not.toBe('');
    expect(THAI.test(line), `the line is worded in Thai: "${line}"`).toBe(true);
    expect(line).toContain(INVALID_SUFFIX);
    expect(line).not.toContain(serverText);
    expect(onSaved).not.toHaveBeenCalled();
  });
});
