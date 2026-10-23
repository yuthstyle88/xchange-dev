// #4911 (input-validation audit T12): the pricing screens check what the server will refuse before sending, and show
// every refusal as a worded line at the input it is about. Written before the implementation by the sub-controller
// claude/studio-claude-4. Do not edit, delete or weaken these tests.
//
// [required] by these tests:
// - "A line at an input" = an element `[data-field-error]` with non-empty text that is a sibling of that input (same
//   parentElement). The line is worded (Thai under the default th locale) and never the server's raw text.
// - Before sending (ticket §2.1): prices via `checkDecimal(text, RATE)`, margins via `checkDecimal(text, RATE_PERCENT)`,
//   min spread via `RATE_PERCENT` and 0 ≤ v ≤ 5, reason non-blank after trim. A failure sends nothing.
// - Server refusals (ticket §2.2, BE contract #4902 §2): `400 validationFailed` with `field` = `reason`,
//   `tiers[<i>].buy|sell` (0-based tier index, so tier n is `tiers[n-1]`), `margins.<name>` or `minSpreadPct`, routed
//   through `fieldError`, so `tiers[1].sell` shows `input.invalid` ("… ไม่ถูกต้อง") at tier 2's sell input.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { makePricingApproveOutput } from '../../../shared/testing/fixtures/pricing_approve';
import { makePricingConfigureOutput } from '../../../shared/testing/fixtures/pricing_configure';
import type { PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { BulkApprove } from './BulkApprove';
import { PricingPolicy } from './PricingPolicy';
import { ROUTE_PRICING_APPROVE, ROUTE_PRICING_CONFIGURE } from './pricing-labels';
import { TierEditor } from './TierEditor';

const USD = BOARD.rows.find((r) => r.currency === 'USD')!;
const THAI = /[฀-๿]/;
const INVALID_SUFFIX = sharedTh['input.invalid'].replace('{field}', '').trim();

type Command = (path: string, body: unknown) => { status: number; body: unknown };

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mountHost(): HTMLDivElement {
  host = document.createElement('div');
  document.body.appendChild(host);
  return host;
}

function approveOk(): Command {
  const out = makePricingApproveOutput({ policyVersionId: 'pv-7', version: 2, draftId: 'draft-9', draftState: 'PUBLISHED' });
  return (path) => (path === ROUTE_PRICING_APPROVE ? { status: 200, body: out } : { status: 404, body: {} });
}

function configureOk(): Command {
  const out = makePricingConfigureOutput({ id: 'pv-8', version: 2 });
  return (path) => (path === ROUTE_PRICING_CONFIGURE ? { status: 200, body: out } : { status: 404, body: {} });
}

function mountTierEditor(row: PricingRowDto, command: Command = approveOk()) {
  const server = boardFetch([BOARD], command);
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  dispose = render(
    () => <TierEditor api={api} branch="MAIN" policyVersionId="pv-7" row={row} bands={BOARD.bands} onApproved={vi.fn()} />,
    mountHost(),
  );
  return server;
}

function mountBulk(rows: PricingRowDto[]) {
  const server = boardFetch([BOARD], approveOk());
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  dispose = render(
    () => <BulkApprove api={api} branch="MAIN" policyVersionId="pv-7" rows={rows} onApproved={vi.fn()} />,
    mountHost(),
  );
  return server;
}

function mountPolicy(row: PricingRowDto) {
  const server = boardFetch([BOARD], configureOk());
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  dispose = render(
    () => <PricingPolicy api={api} branch="MAIN" policyVersionId="pv-7" row={row} mayEdit onSaved={vi.fn()} />,
    mountHost(),
  );
  return server;
}

const posts = (server: ReturnType<typeof boardFetch>, path: string) =>
  server.calls.filter((c) => c.method === 'POST' && c.path === path);

function input(name: string): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `input[name=${name}] exists`).not.toBeNull();
  return el!;
}

function type(name: string, value: string) {
  const el = input(name);
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

function click(name: string) {
  const b = host.querySelector<HTMLButtonElement>(`button[name="${name}"]`);
  expect(b, `button[name=${name}] exists`).not.toBeNull();
  b!.click();
}

/** The worded line at an input: a `[data-field-error]` sibling (same parentElement) with text. */
function lineAt(name: string): string {
  const el = input(name);
  return el.parentElement?.querySelector('[data-field-error]')?.textContent?.trim() ?? '';
}

function expectWordedLineAt(name: string) {
  const text = lineAt(name);
  expect(text, `a worded line at input[name=${name}]`).not.toBe('');
  expect(THAI.test(text), `the line at ${name} is worded in Thai: "${text}"`).toBe(true);
}

describe('#4911 pricing screens: checked before sending, refusals at the field', () => {
  it('E1: TierEditor, blank reason then Approve → worded line at the reason; nothing is sent', async () => {
    const server = mountTierEditor(USD);
    type('reason', '   ');
    click('approve');
    await flush();
    expect(posts(server, ROUTE_PRICING_APPROVE), 'no pricingApprove for a blank reason').toHaveLength(0);
    expectWordedLineAt('reason');
  });

  it('E2: TierEditor, tier 1 buy 1.123456789 (9 decimals > RATE scale 8) → worded line at that buy; nothing is sent', async () => {
    const server = mountTierEditor(USD);
    type('buy-1', '1.123456789');
    type('reason', 'adjust tier 1 buy');
    click('approve');
    await flush();
    expect(posts(server, ROUTE_PRICING_APPROVE), 'no pricingApprove for a 9-decimal price').toHaveLength(0);
    expectWordedLineAt('buy-1');
    expect(lineAt('sell-1'), 'no line at an input that is fine').toBe('');
  });

  it('E3: BulkApprove, confirm with a blank reason → worded line at the reason; nothing is sent', async () => {
    const server = mountBulk([USD]);
    click('bulkApprove');
    await flush();
    type('reason', '  ');
    click('confirmBulk');
    await flush();
    expect(posts(server, ROUTE_PRICING_APPROVE), 'no pricingApprove for a blank reason').toHaveLength(0);
    expectWordedLineAt('reason');
  });

  it('E4: PricingPolicy, tier 1 buy margin 1.23456 (5 decimals > RATE_PERCENT scale 4) → worded line; nothing is sent', async () => {
    const server = mountPolicy(USD);
    type('buy-margin-1', '1.23456');
    type('reason', 'tighten tier 1');
    click('save');
    await flush();
    expect(posts(server, ROUTE_PRICING_CONFIGURE), 'no pricingConfigure for a 5-decimal margin').toHaveLength(0);
    expectWordedLineAt('buy-margin-1');
  });

  it('E5: PricingPolicy, min spread 6 (above 5) → worded line at min spread; nothing is sent', async () => {
    const server = mountPolicy(USD);
    type('minSpreadPct', '6');
    type('reason', 'widen spread');
    click('save');
    await flush();
    expect(posts(server, ROUTE_PRICING_CONFIGURE), 'no pricingConfigure for a min spread above 5').toHaveLength(0);
    expectWordedLineAt('minSpreadPct');
  });

  it('E6: TierEditor, a server 400 validationFailed at field tiers[1].sell → input.invalid line at tier 2 sell, not the raw text', async () => {
    const raw = 'tier 2 sell must be above buy';
    const server = mountTierEditor(USD, (path) =>
      path === ROUTE_PRICING_APPROVE
        ? { status: 400, body: { error: { code: 'validationFailed', field: 'tiers[1].sell', message: raw } } }
        : { status: 404, body: {} },
    );
    type('reason', 'approve as suggested');
    click('approve');
    await flush();
    expect(posts(server, ROUTE_PRICING_APPROVE), 'the valid request is sent once').toHaveLength(1);
    const text = lineAt('sell-2');
    expect(text, 'a line at tier 2 sell').not.toBe('');
    expect(text.endsWith(INVALID_SUFFIX), `input.invalid wording at tier 2 sell: "${text}"`).toBe(true);
    expect(host.textContent ?? '', 'the raw server text is never shown').not.toContain(raw);
    expect(lineAt('sell-1'), 'tier 1 sell carries no line').toBe('');
  });
});
