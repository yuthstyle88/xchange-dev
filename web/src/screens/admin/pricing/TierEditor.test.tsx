// §9 `TierEditor` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §2, §5.2, §6, §9). Written before the
// implementation by the sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests.
//
// [required] by these tests:
// - Inputs `input[name=buy-<n>]` / `input[name=sell-<n>]` (n = 1..4) prefilled with `row.suggestion.tiers` as sent.
// - A margin cell `[data-margin=buy-<n>]` / `[data-margin=sell-<n>]` shows `row.suggestion.margins` as sent. After the
//   user edits that tier's price it shows the `marginOnApprove` text instead: the server stores the margin a price makes
//   (§2, §5.2), and the page computes no figure (W7).
// - `button[name=approve]` sends ONE `POST /api/pricing/approve` with body
//   `{ branch, policyVersionId, currencies: [{ currency, tiers: [4 × {tierNo, buy, sell}] }] }` and an `Idempotency-Key`
//   header (`keyedWrite`; design §6's `requestId` is that header, rates.md §5.6)
//   carrying the edited prices, then calls `onApproved` with the answer.
// - A row without a suggestion: empty inputs, approve disabled.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { th } from './i18n';
import { TierEditor } from './TierEditor';
import type { PricingRowView } from './pricing-board.types';

const USD = BOARD.rows.find((r) => r.currency === 'USD')!;
const KRW = BOARD.rows.find((r) => r.currency === 'KRW')!;

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
function mount(row: PricingRowView, onApproved = vi.fn()) {
  const server = boardFetch([BOARD], (path) =>
    path === '/api/pricing/approve' ? { status: 200, body: { draftId: 'draft-9', state: 'PUBLISHED' } } : { status: 404, body: {} },
  );
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <TierEditor api={api} branch="MAIN" policyVersionId="pv-7" row={row} bands={BOARD.bands} onApproved={onApproved} />,
    host,
  );
  return { server, onApproved };
}
const input = (name: string) => host.querySelector<HTMLInputElement>(`input[name=${name}]`);
const marginCell = (key: string) => host.querySelector(`[data-margin=${key}]`)?.textContent ?? '';
function edit(name: string, value: string) {
  const i = input(name);
  expect(i, `input[name=${name}] exists`).not.toBeNull();
  i!.value = value;
  i!.dispatchEvent(new Event('input', { bubbles: true }));
}
function click(name: string) {
  const b = host.querySelector<HTMLButtonElement>(`button[name=${name}]`);
  expect(b, `button[name=${name}] exists`).not.toBeNull();
  b!.click();
}

describe('TierEditor', () => {
  it('E1 prefills the 4 buy and sell prices from the suggestion, as sent', () => {
    mount(USD);
    for (const t of USD.suggestion!.tiers) {
      expect(input(`buy-${t.tierNo}`)?.value).toBe(t.buy);
      expect(input(`sell-${t.tierNo}`)?.value).toBe(t.sell);
    }
  });

  it('E2 shows the margin each suggested price makes, as sent', () => {
    mount(USD);
    for (const m of USD.suggestion!.margins) {
      expect(marginCell(`buy-${m.tierNo}`)).toContain(m.buyMarginPct);
      expect(marginCell(`sell-${m.tierNo}`)).toContain(m.sellMarginPct);
    }
  });

  it('E3 an edited price shows "computed on approve" for its margin, not the old figure', () => {
    mount(USD);
    edit('buy-2', '35.09');
    expect(marginCell('buy-2')).toContain(th.marginOnApprove);
    expect(marginCell('buy-2')).not.toContain(USD.suggestion!.margins[1].buyMarginPct);
    expect(marginCell('buy-1')).toContain(USD.suggestion!.margins[0].buyMarginPct);
  });

  it('E4 approve sends one pricingApprove with the edited prices, an Idempotency-Key and policyVersionId', async () => {
    const { server, onApproved } = mount(USD);
    edit('buy-2', '35.09');
    click('approve');
    await flush();
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.branch).toBe('MAIN');
    expect(body.policyVersionId).toBe('pv-7');
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
    expect('requestId' in body).toBe(false);
    expect(body.currencies).toHaveLength(1);
    expect(body.currencies[0].currency).toBe('USD');
    const tiers = body.currencies[0].tiers as { tierNo: number; buy: string; sell: string }[];
    expect(tiers.map((t) => t.tierNo)).toEqual([1, 2, 3, 4]);
    expect(tiers[1].buy).toBe('35.09');
    expect(tiers[0].buy).toBe(USD.suggestion!.tiers[0].buy);
    expect(onApproved).toHaveBeenCalledWith({ draftId: 'draft-9', state: 'PUBLISHED' });
  });

  it('E5 a row without a suggestion has empty prices and approve disabled', () => {
    mount(KRW);
    expect(input('buy-1')?.value ?? null).toBe('');
    expect(host.querySelector<HTMLButtonElement>('button[name=approve]')?.disabled).toBe(true);
  });
});
