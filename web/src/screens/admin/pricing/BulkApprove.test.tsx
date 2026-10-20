// §9 `BulkApprove` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §5.1 blocking flags, §6, §11).
// Written before the implementation by the sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests.
//
// [required] by these tests:
// - Rows carrying a blocking flag (`BLOCKING_FLAGS`: BelowCost, StaleSource, Precision) are never sent; `[data-count]`
//   shows how many rows can be approved.
// - `button[name=bulkApprove]` asks for a confirmation; `button[name=confirmBulk]` sends ONE `POST /api/pricing/approve`
//   with `{ policyVersionId, rows, reason }` and an `Idempotency-Key` header (`keyedWrite`; §6's `requestId`), one entry per approvable row in the given
//   order, each with that row's suggested tier prices; then `onApproved` with the answer.
// - No approvable row: `bulkApprove` disabled, nothing sent.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { makePricingApproveOutput } from '../../../shared/testing/fixtures/pricing_approve';
import type { PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { BulkApprove } from './BulkApprove';

const pick = (...codes: string[]) => codes.map((c) => BOARD.rows.find((r) => r.currency === c)!);

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
function mount(rows: PricingRowDto[], onApproved = vi.fn()) {
  const approvedOut = makePricingApproveOutput({
    policyVersionId: 'pv-7',
    version: 2,
    draftId: 'draft-10',
    draftState: 'PENDING_APPROVAL',
  });
  const server = boardFetch([BOARD], () => ({ status: 200, body: approvedOut }));
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <BulkApprove api={api} branch="MAIN" policyVersionId="pv-7" rows={rows} onApproved={onApproved} />,
    host,
  );
  return { server, onApproved };
}
function click(name: string) {
  const b = host.querySelector<HTMLButtonElement>(`button[name=${name}]`);
  expect(b, `button[name=${name}] exists`).not.toBeNull();
  b!.click();
}
const approves = (s: ReturnType<typeof boardFetch>) => s.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');

describe('BulkApprove', () => {
  it('B1 counts only rows without a blocking flag', () => {
    mount(pick('USD', 'EUR', 'JPY', 'GBP', 'AUD'));
    expect(host.querySelector('[data-count]')?.getAttribute('data-count')).toBe('2');
  });

  it('B2 confirm sends one pricingApprove with the approvable rows only, an Idempotency-Key and policyVersionId', async () => {
    const { server, onApproved } = mount(pick('USD', 'EUR', 'JPY', 'GBP', 'AUD'));
    click('bulkApprove');
    await flush();
    expect(approves(server)).toHaveLength(0);
    const reasonInput = host.querySelector<HTMLInputElement>('input[name=reason]');
    expect(reasonInput, 'input[name=reason] exists').not.toBeNull();
    reasonInput!.value = 'morning market update';
    reasonInput!.dispatchEvent(new Event('input', { bubbles: true }));
    click('confirmBulk');
    await flush();
    const sent = approves(server);
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.policyVersionId).toBe('pv-7');
    expect(body.reason).toBe('morning market update');
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
    expect('requestId' in body).toBe(false);
    expect(body.rows.map((c: { currency: string }) => c.currency)).toEqual(['USD', 'AUD']);
    const usd = BOARD.rows.find((r) => r.currency === 'USD')!;
    expect(body.rows[0].tiers).toEqual(usd.suggestion);
    expect(onApproved).toHaveBeenCalledWith(
      makePricingApproveOutput({
        policyVersionId: 'pv-7',
        version: 2,
        draftId: 'draft-10',
        draftState: 'PENDING_APPROVAL',
      }),
    );
  });

  it('B3 with only blocked rows, bulk approve is disabled and nothing is sent', async () => {
    const { server } = mount(pick('EUR', 'JPY', 'GBP'));
    const b = host.querySelector<HTMLButtonElement>('button[name=bulkApprove]');
    expect(b?.disabled).toBe(true);
    b?.click();
    await flush();
    expect(approves(server)).toHaveLength(0);
  });
});
