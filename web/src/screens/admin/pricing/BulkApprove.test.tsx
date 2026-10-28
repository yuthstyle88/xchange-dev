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
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import {
  makePricingApproveBranchResult,
  makePricingApproveCurrencyResult,
  makePricingApproveOutput,
} from '../../../shared/testing/fixtures/pricing_approve';
import { makePricingRowDto, makePricingTierSetDto, type PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { BulkApprove } from './BulkApprove';
import { th } from './i18n';

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
    currencies: [
      makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-10' }),
      makePricingApproveCurrencyResult({ currency: 'AUD', state: 'PUBLISHED', tierSetId: 'ts-11' }),
    ],
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
        currencies: [
          makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-10' }),
          makePricingApproveCurrencyResult({ currency: 'AUD', state: 'PUBLISHED', tierSetId: 'ts-11' }),
        ],
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

// #5197 (V.9 PV-c answer). Written before the implementation by claude/8be3a451 (XC FE lane controller).
// Do not edit, delete or weaken these tests.
// [required] by these tests (ticket #5197 §3):
// - Each approve row carries its own `tierSetId` = `rows[i].tierSet?.id`.
// - After a 200 the editor renders `ApproveResults` for the answer (one line per currency: code + `approveStateLabel`;
//   at MAIN, under a currency with `branches`, the `branchPushTitle` heading and one line per branch: `branchId` +
//   `branchPushStateLabel`, a FAILED line adds its worded error) and still calls `onApproved(out)` once.
// - On 409 `staleVersion` it shows `t('approveStale')` and does not call `onApproved`.
describe('#5197 BulkApprove approve answer', () => {
  type Answer = { status: number; body: unknown };
  const ok = (...currencies: ReturnType<typeof makePricingApproveCurrencyResult>[]): Answer => ({
    status: 200,
    body: makePricingApproveOutput({ currencies }),
  });
  function mountAnswer(rows: (typeof BOARD.rows)[number][], answer: Answer, onApproved = vi.fn()) {
    const server = boardFetch([BOARD], (path) => (path === '/api/pricing/approve' ? answer : { status: 404, body: {} }));
    const api = new ApiClient(server.fn as unknown as typeof fetch);
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => <BulkApprove api={api} branch="MAIN" policyVersionId="pv-7" rows={rows} onApproved={onApproved} />,
      host,
    );
    return { server, onApproved };
  }
  async function confirm() {
    click('bulkApprove');
    await flush();
    const reasonInput = host.querySelector<HTMLInputElement>('input[name=reason]');
    expect(reasonInput, 'input[name=reason] exists').not.toBeNull();
    reasonInput!.value = 'morning market update';
    reasonInput!.dispatchEvent(new Event('input', { bubbles: true }));
    click('confirmBulk');
    await flush();
  }
  /** The smallest element whose text holds both `needle` and `label`. */
  function lineOf(needle: string, label: string): string {
    const hits = Array.from(host.querySelectorAll('*')).filter((el) => {
      const text = el.textContent ?? '';
      return text.includes(needle) && text.includes(label);
    });
    const line = hits.find((el) => !hits.some((o) => o !== el && el.contains(o)));
    expect(line, `a line holding "${needle}" and "${label}"`).toBeTruthy();
    return line!.textContent ?? '';
  }
  const [USD, AUD] = pick('USD', 'AUD');
  const published = (currency: string, tierSetId: string) =>
    makePricingApproveCurrencyResult({ currency, state: 'PUBLISHED', tierSetId });

  it('R3 two rows with different tier sets: each approve row carries its own tierSetId', async () => {
    const rows = [
      makePricingRowDto({ ...USD, tierSet: makePricingTierSetDto({ id: 'ts-1' }) }),
      makePricingRowDto({ ...AUD, tierSet: makePricingTierSetDto({ id: 'ts-2' }) }),
    ];
    const { server } = mountAnswer(rows, ok(published('USD', 'ts-10'), published('AUD', 'ts-11')));
    await confirm();
    const sent = approves(server);
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.policyVersionId).toBe('pv-7');
    expect(body.rows.map((r: { currency: string; tierSetId?: string }) => [r.currency, r.tierSetId])).toEqual([
      ['USD', 'ts-1'],
      ['AUD', 'ts-2'],
    ]);
  });

  it('R4 200 with every currency PUBLISHED, no branches: one line per currency reading "ประกาศแล้ว"; onApproved called once', async () => {
    const answer = ok(published('USD', 'ts-10'), published('AUD', 'ts-11'));
    const { onApproved } = mountAnswer([USD, AUD], answer);
    expect(host.textContent ?? '', 'no result before approving').not.toContain(th['approveState.PUBLISHED']);
    await confirm();
    expect(lineOf('USD', 'ประกาศแล้ว'), 'the USD line holds only USD').not.toContain('AUD');
    expect(lineOf('AUD', 'ประกาศแล้ว'), 'the AUD line holds only AUD').not.toContain('USD');
    expect((host.textContent ?? '').split('ประกาศแล้ว').length - 1, 'one line per currency').toBe(2);
    expect(host.textContent ?? '', 'no branch heading without branches').not.toContain(th.branchPushTitle);
    expect(onApproved).toHaveBeenCalledTimes(1);
    expect(onApproved).toHaveBeenCalledWith(answer.body);
  });

  it('R6 200 at MAIN with branches PUBLISHED / SKIPPED_OWN / FAILED / ALREADY_DONE: four branch lines, FAILED worded', async () => {
    const answer = ok(
      makePricingApproveCurrencyResult({
        currency: 'USD',
        state: 'PUBLISHED',
        tierSetId: 'ts-10',
        branches: [
          makePricingApproveBranchResult({ branchId: 'SIAM', state: 'PUBLISHED', tierSetId: 'ts-31' }),
          makePricingApproveBranchResult({ branchId: 'CNX', state: 'SKIPPED_OWN' }),
          makePricingApproveBranchResult({ branchId: 'HKT', state: 'FAILED', error: 'serverError' }),
          makePricingApproveBranchResult({ branchId: 'PTY', state: 'ALREADY_DONE', tierSetId: 'ts-34' }),
        ],
      }),
    );
    const { onApproved } = mountAnswer([USD], answer);
    await confirm();
    expect(host.textContent ?? '', 'the branchPushTitle heading').toContain(th.branchPushTitle);
    const ids = ['SIAM', 'CNX', 'HKT', 'PTY'];
    const labels = [th['branchPush.PUBLISHED'], th['branchPush.SKIPPED_OWN'], th['branchPush.FAILED'], th['branchPush.ALREADY_DONE']];
    const lines = ids.map((id, i) => {
      const line = lineOf(id, labels[i]!);
      for (const other of ids.filter((o) => o !== id)) expect(line, `the ${id} line holds only ${id}`).not.toContain(other);
      return line;
    });
    const failed = wordedError(new ApiError({ status: 409, code: 'serverError', message: '' }));
    expect(lines[2], 'the FAILED line shows its worded error').toContain(failed);
    for (const i of [0, 1, 3]) expect(lines[i], 'only the FAILED line carries an error').not.toContain(failed);
    expect(onApproved).toHaveBeenCalledTimes(1);
  });

  it('R7 409 staleVersion: shows the approveStale text and does not call onApproved', async () => {
    const { server, onApproved } = mountAnswer([USD, AUD], {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'tier set moved on (test)' } },
    });
    await confirm();
    expect(approves(server)).toHaveLength(1);
    expect(host.textContent ?? '', 'the approveStale text').toContain(th.approveStale);
    expect(onApproved).not.toHaveBeenCalled();
  });
});
