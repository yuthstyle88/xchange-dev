// §9 `TierEditor` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §2, §5.2, §6, §9). Written before the
// implementation by the sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests.
//
// [required] by these tests:
// - Inputs `input[name=buy-<n>]` / `input[name=sell-<n>]` (n = 1..4) prefilled with `row.suggestion` as sent.
// - A margin cell `[data-margin=buy-<n>]` / `[data-margin=sell-<n>]` shows `row.margins` as sent. After the
//   user edits that tier's price it shows the `marginOnApprove` text instead: the server stores the margin a price makes
//   (§2, §5.2), and the page computes no figure (W7).
// - `button[name=approve]` sends ONE `POST /api/pricing/approve` with body
//   `{ policyVersionId, rows: [{ currency, tiers: [4 × {tierNo, buy, sell}] }], reason }` and an `Idempotency-Key`
//   header (`keyedWrite`; design §6's `requestId` is that header, rates.md §5.6)
//   carrying the edited prices, then calls `onApproved` with the answer.
// - A row without a suggestion: empty inputs, approve disabled.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import {
  makePricingApproveCurrencyResult,
  makePricingApproveOutput,
} from '../../../shared/testing/fixtures/pricing_approve';
import { makePricingRowDto, makePricingTierSetDto, type PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { th } from './i18n';
import { TierEditor } from './TierEditor';

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
function mount(row: PricingRowDto, onApproved = vi.fn()) {
  const approvedOut = makePricingApproveOutput({
    currencies: [makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-9' })],
  });
  const server = boardFetch([BOARD], (path) =>
    path === '/api/pricing/approve' ? { status: 200, body: approvedOut } : { status: 404, body: {} },
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
    for (const t of USD.suggestion!) {
      expect(input(`buy-${t.tierNo}`)?.value).toBe(t.buy);
      expect(input(`sell-${t.tierNo}`)?.value).toBe(t.sell);
    }
  });

  it('E2 shows the row effective margin for each tier, as sent', () => {
    mount(USD);
    for (const m of USD.margins) {
      expect(marginCell(`buy-${m.tierNo}`)).toContain(m.buyMarginPct);
      expect(marginCell(`sell-${m.tierNo}`)).toContain(m.sellMarginPct);
    }
  });

  it('E3 an edited price shows "computed on approve" for its margin, not the old figure', () => {
    mount(USD);
    edit('buy-2', '35.09');
    expect(marginCell('buy-2')).toContain(th.marginOnApprove);
    expect(marginCell('buy-2')).not.toContain(USD.margins[1].buyMarginPct);
    expect(marginCell('buy-1')).toContain(USD.margins[0].buyMarginPct);
  });

  it('E4 approve sends one pricingApprove with the edited prices, an Idempotency-Key and policyVersionId', async () => {
    const { server, onApproved } = mount(USD);
    edit('buy-2', '35.09');
    edit('reason', 'adjust tier 2 buy');
    click('approve');
    await flush();
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.policyVersionId).toBe('pv-7');
    expect(body.reason).toBe('adjust tier 2 buy');
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
    expect('requestId' in body).toBe(false);
    expect(body.rows).toHaveLength(1);
    expect(body.rows[0].currency).toBe('USD');
    const tiers = body.rows[0].tiers as { tierNo: number; buy: string; sell: string }[];
    expect(tiers.map((t) => t.tierNo)).toEqual([1, 2, 3, 4]);
    expect(tiers[1].buy).toBe('35.09');
    expect(tiers[0].buy).toBe(USD.suggestion![0].buy);
    expect(onApproved).toHaveBeenCalledWith(
      makePricingApproveOutput({
        currencies: [makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-9' })],
      }),
    );
  });

  it('E5 a row without a suggestion has empty prices and approve disabled', () => {
    mount(KRW);
    expect(input('buy-1')?.value ?? null).toBe('');
    expect(host.querySelector<HTMLButtonElement>('button[name=approve]')?.disabled).toBe(true);
  });
});

// #5197 (V.9 PV-c answer). Written before the implementation by claude/8be3a451 (XC FE lane controller).
// Do not edit, delete or weaken these tests.
// [required] by these tests (ticket #5197 §3):
// - Each approve row carries `tierSetId` = `row.tierSet?.id`; the key is omitted when `row.tierSet` is null.
// - After a 200 the editor renders `ApproveResults` for the answer (one line per currency: code + `approveStateLabel`,
//   a REFUSED line adds the `wordedError` of its code) and still calls `onApproved(out)` once.
// - On 409 `staleVersion` it shows `t('approveStale')` and does not call `onApproved`.
describe('#5197 TierEditor approve answer', () => {
  type Answer = { status: number; body: unknown };
  const ok = (...currencies: ReturnType<typeof makePricingApproveCurrencyResult>[]): Answer => ({
    status: 200,
    body: makePricingApproveOutput({ currencies }),
  });
  function mountAnswer(row: typeof USD, answer: Answer, onApproved = vi.fn()) {
    const server = boardFetch([BOARD], (path) => (path === '/api/pricing/approve' ? answer : { status: 404, body: {} }));
    const api = new ApiClient(server.fn as unknown as typeof fetch);
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => <TierEditor api={api} branch="MAIN" policyVersionId="pv-7" row={row} bands={BOARD.bands} onApproved={onApproved} />,
      host,
    );
    return { server, onApproved };
  }
  async function approve() {
    edit('reason', 'approve as suggested');
    click('approve');
    await flush();
  }
  const sentRows = (server: ReturnType<typeof boardFetch>) => {
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    return (sent[0].body as Record<string, any>).rows as Record<string, unknown>[];
  };
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
  const usdPublished = makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-10' });

  it('R1 a row with tierSet.id "ts-9": the approve body row carries tierSetId "ts-9"', async () => {
    const row = makePricingRowDto({ ...USD, tierSet: makePricingTierSetDto({ id: 'ts-9' }) });
    const { server } = mountAnswer(row, ok(usdPublished));
    await approve();
    const rows = sentRows(server);
    expect(rows).toHaveLength(1);
    expect(rows[0].currency).toBe('USD');
    expect(rows[0].tierSetId).toBe('ts-9');
  });

  it('R2 a row with tierSet null: the approve body row has no tierSetId key', async () => {
    expect(USD.tierSet, 'the USD board row has no tier set').toBeNull();
    const { server } = mountAnswer(USD, ok(usdPublished));
    await approve();
    const rows = sentRows(server);
    expect(rows[0].currency).toBe('USD');
    expect('tierSetId' in rows[0], 'no tierSetId key').toBe(false);
  });

  it('R4 200 with USD PUBLISHED: one USD line reading "ประกาศแล้ว"; onApproved called once with the answer', async () => {
    const answer = ok(usdPublished);
    const { onApproved } = mountAnswer(USD, answer);
    expect(host.textContent ?? '', 'no result before approving').not.toContain(th['approveState.PUBLISHED']);
    await approve();
    expect(th['approveState.PUBLISHED']).toBe('ประกาศแล้ว');
    lineOf('USD', 'ประกาศแล้ว');
    expect((host.textContent ?? '').split('ประกาศแล้ว').length - 1, 'one line for the one currency').toBe(1);
    expect(onApproved).toHaveBeenCalledTimes(1);
    expect(onApproved).toHaveBeenCalledWith(answer.body);
  });

  it('R5 200 with USD REFUSED approvalRequired: the line reads "ไม่ผ่าน" + the worded error; onApproved still called', async () => {
    const answer = ok(makePricingApproveCurrencyResult({ currency: 'USD', state: 'REFUSED', tierSetId: null, error: 'approvalRequired' }));
    const { onApproved } = mountAnswer(USD, answer);
    await approve();
    expect(th['approveState.REFUSED']).toBe('ไม่ผ่าน');
    const line = lineOf('USD', 'ไม่ผ่าน');
    expect(line, 'the pricing refused text (#5215)').toContain(th['approveRefused.approvalRequired']);
    expect(line, 'never the raw code').not.toContain('approvalRequired');
    expect(onApproved).toHaveBeenCalledTimes(1);
    expect(onApproved).toHaveBeenCalledWith(answer.body);
  });

  it('R7 409 staleVersion: shows the approveStale text and does not call onApproved', async () => {
    const { server, onApproved } = mountAnswer(USD, {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'tier set moved on (test)' } },
    });
    await approve();
    expect(sentRows(server)).toHaveLength(1);
    expect(host.textContent ?? '', 'the approveStale text').toContain(th.approveStale);
    expect(onApproved).not.toHaveBeenCalled();
  });
});
