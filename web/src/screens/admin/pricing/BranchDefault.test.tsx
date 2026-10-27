// #5200 (V.9 PV-d2): `BranchDefault` edits the branch default bp (the `"*"` row) and saves it with one keyed
// POST /api/pricing/branch-policy `{currency: "*", buyBp, sellBp}`. Up-front tests, written before the implementation.
// Do not edit, delete or weaken these tests.
//
// [required] by these tests (ticket #5200 §3 and its behaviour table D1..D3):
// - Inputs `input[name="buyBp"]`, `input[name="sellBp"]`; save is `button[name="save-default"]`.
// - "A line at an input" = an element `[data-field-error]` with non-empty text that is a sibling of that input (same
//   parentElement), as in `Pricing.input-validation.test.tsx` and `PricingPolicy.tsx`.
// - bp is a whole number with BRANCH_BP_MIN <= bp <= BRANCH_BP_MAX; anything else shows `bpRange` ({min}, {max}) at
//   that input and posts nothing.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { makePricingBpDto } from '../../../shared/testing/fixtures/pricing_board';
import {
  makePricingBranchPolicySaveOutput,
  type PricingBranchPolicySaveOutput,
} from '../../../shared/testing/fixtures/pricing_branch_policy_save';
import { boardFetch } from './__fixtures__/pricing-board.fixture';
import { BranchDefault } from './BranchDefault';
import { th as pricingTh } from './i18n';
import { BRANCH_BP_MAX, BRANCH_BP_MIN, ROUTE_PRICING_BRANCH_POLICY } from './pricing-labels';

const BP_RANGE = pricingTh.bpRange
  .replace('{min}', String(BRANCH_BP_MIN))
  .replace('{max}', String(BRANCH_BP_MAX));

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

function mount(opts?: { mayEdit?: boolean }) {
  const onSaved = vi.fn<(out: PricingBranchPolicySaveOutput) => void>();
  const server = boardFetch(undefined, (path) =>
    path === ROUTE_PRICING_BRANCH_POLICY
      ? { status: 200, body: makePricingBranchPolicySaveOutput({ saved: true }) }
      : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } },
  );
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  const el = document.createElement('div');
  host = el;
  document.body.appendChild(el);
  dispose = render(
    () => (
      <BranchDefault
        api={api}
        branch="B2"
        value={makePricingBpDto({ buyBp: 0, sellBp: 20 })}
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
  const b = host!.querySelector<HTMLButtonElement>('button[name="save-default"]');
  expect(b, 'button[name=save-default] exists').not.toBeNull();
  b!.click();
  await flush();
}

/** The worded line at an input: a `[data-field-error]` sibling (same parentElement) with text. */
function lineAt(name: string): string {
  return input(name).parentElement?.querySelector('[data-field-error]')?.textContent?.trim() ?? '';
}

const posts = (server: ReturnType<typeof boardFetch>) => server.calls.filter((c) => c.method === 'POST');

describe('BranchDefault (#5200)', () => {
  it('D1 value {buyBp: 0, sellBp: 20}, sellBp 30, save - one keyed POST {currency: "*", buyBp: 0, sellBp: 30}, no own/follow', async () => {
    const { server } = mount();
    expect(input('buyBp').value).toBe('0');
    expect(input('sellBp').value).toBe('20');
    type('sellBp', '30');
    await save();

    const sent = posts(server);
    expect(sent).toHaveLength(1);
    expect(sent[0].path).toBe(ROUTE_PRICING_BRANCH_POLICY);
    expect(sent[0].body).toEqual({ currency: '*', buyBp: 0, sellBp: 30 });
    expect('own' in (sent[0].body as object)).toBe(false);
    expect('follow' in (sent[0].body as object)).toBe(false);
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
  });

  it('D2 sellBp 500 - bpRange at sellBp; no POST', async () => {
    const { server, onSaved } = mount();
    type('sellBp', '500');
    await save();

    expect(lineAt('sellBp')).toBe(BP_RANGE);
    expect(posts(server)).toHaveLength(0);
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('D3 mayEdit false - inputs disabled (§3 read-only); no save-default button', () => {
    mount({ mayEdit: false });
    expect(input('buyBp').disabled).toBe(true);
    expect(input('sellBp').disabled).toBe(true);
    expect(host!.querySelector('button[name="save-default"]')).toBeNull();
  });
});
