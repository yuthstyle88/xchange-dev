// #5197 `ApproveResults` (V.9 PV-c answer; docs/design/blueprint/pricing-tiers.md §15.8a). Written before the
// implementation by claude/8be3a451 (XC FE lane controller). Do not edit, delete or weaken these tests.
//
// [required] by these tests (ticket #5197 §3):
// - `<ApproveResults out={PricingApproveOutput} />` shows one line per currency: the code + `approveStateLabel(state)`;
//   a `REFUSED` line also shows the `wordedError` of its `error` code.
// - Under a currency with `branches`: the `branchPushTitle` heading, then one line per branch: `branchId` +
//   `branchPushStateLabel(state)`; a `FAILED` line also shows its worded `error`.
// - "A line" is not pinned to markup: it is the smallest element holding both the code and its label, and it must
//   not hold another currency's code (or another branch's id).
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import {
  makePricingApproveBranchResult,
  makePricingApproveCurrencyResult,
  makePricingApproveOutput,
} from '../../../shared/testing/fixtures/pricing_approve';
import { ApproveResults } from './ApproveResults';
import { th } from './i18n';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(out: ReturnType<typeof makePricingApproveOutput>) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <ApproveResults out={out} />, host);
}

/** The worded text the page shows for an error code (`wordedError` of that code). */
const worded = (code: string) => wordedError(new ApiError({ status: 409, code, message: '' }));

/** The smallest element whose text holds both `needle` and `label`, or null. */
function lineOf(needle: string, label: string): Element | null {
  const hits = Array.from(host.querySelectorAll('*')).filter((el) => {
    const text = el.textContent ?? '';
    return text.includes(needle) && text.includes(label);
  });
  return hits.find((el) => !hits.some((o) => o !== el && el.contains(o))) ?? null;
}

function expectLine(needle: string, label: string, others: string[]): string {
  const line = lineOf(needle, label);
  expect(line, `a line holding "${needle}" and "${label}"`).not.toBeNull();
  const text = line!.textContent ?? '';
  for (const o of others) expect(text, `the ${needle} line does not hold ${o}`).not.toContain(o);
  return text;
}

const count = (needle: string) => (host.textContent ?? '').split(needle).length - 1;

describe('#5197 ApproveResults', () => {
  it('R4 every currency PUBLISHED, no branches: one line per currency reading "ประกาศแล้ว", no branch heading', () => {
    mount(
      makePricingApproveOutput({
        currencies: [
          makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-21' }),
          makePricingApproveCurrencyResult({ currency: 'EUR', state: 'PUBLISHED', tierSetId: 'ts-22' }),
        ],
      }),
    );
    expect(th['approveState.PUBLISHED']).toBe('ประกาศแล้ว');
    expectLine('USD', 'ประกาศแล้ว', ['EUR']);
    expectLine('EUR', 'ประกาศแล้ว', ['USD']);
    expect(count('ประกาศแล้ว'), 'one PUBLISHED label per currency').toBe(2);
    expect(host.textContent ?? '', 'no branch heading without branches').not.toContain(th.branchPushTitle);
  });

  it('R5 a REFUSED currency with error approvalRequired: its line reads "ไม่ผ่าน" + the worded error', () => {
    mount(
      makePricingApproveOutput({
        currencies: [
          makePricingApproveCurrencyResult({ currency: 'USD', state: 'PUBLISHED', tierSetId: 'ts-21' }),
          makePricingApproveCurrencyResult({ currency: 'EUR', state: 'REFUSED', tierSetId: null, error: 'approvalRequired' }),
        ],
      }),
    );
    expect(th['approveState.REFUSED']).toBe('ไม่ผ่าน');
    const eur = expectLine('EUR', 'ไม่ผ่าน', ['USD']);
    expect(eur, 'the REFUSED line shows the worded error').toContain(worded('approvalRequired'));
    expect(eur, 'never the raw code').not.toContain('approvalRequired');
    const usd = expectLine('USD', 'ประกาศแล้ว', ['EUR']);
    expect(usd, 'the PUBLISHED line shows no refusal').not.toContain('ไม่ผ่าน');
  });

  it('R6 at MAIN, branches PUBLISHED / SKIPPED_OWN / FAILED / ALREADY_DONE: heading + four branch lines, FAILED worded', () => {
    const ids = ['SIAM', 'CNX', 'HKT', 'PTY'];
    mount(
      makePricingApproveOutput({
        currencies: [
          makePricingApproveCurrencyResult({
            currency: 'USD',
            state: 'PUBLISHED',
            tierSetId: 'ts-21',
            branches: [
              makePricingApproveBranchResult({ branchId: 'SIAM', state: 'PUBLISHED', tierSetId: 'ts-31' }),
              makePricingApproveBranchResult({ branchId: 'CNX', state: 'SKIPPED_OWN' }),
              makePricingApproveBranchResult({ branchId: 'HKT', state: 'FAILED', error: 'serverError' }),
              makePricingApproveBranchResult({ branchId: 'PTY', state: 'ALREADY_DONE', tierSetId: 'ts-34' }),
            ],
          }),
        ],
      }),
    );
    expect(host.textContent ?? '', 'the branchPushTitle heading').toContain(th.branchPushTitle);
    const others = (id: string) => ids.filter((o) => o !== id);
    const failedText = worded('serverError');
    const siam = expectLine('SIAM', th['branchPush.PUBLISHED'], others('SIAM'));
    const cnx = expectLine('CNX', th['branchPush.SKIPPED_OWN'], others('CNX'));
    const hkt = expectLine('HKT', th['branchPush.FAILED'], others('HKT'));
    const pty = expectLine('PTY', th['branchPush.ALREADY_DONE'], others('PTY'));
    expect(hkt, 'the FAILED line shows its worded error').toContain(failedText);
    expect(hkt, 'never the raw code').not.toContain('serverError');
    for (const ok of [siam, cnx, pty]) expect(ok, 'only the FAILED line carries an error').not.toContain(failedText);
  });
});
