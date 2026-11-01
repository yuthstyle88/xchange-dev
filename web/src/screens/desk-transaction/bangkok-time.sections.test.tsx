// FE-T6 #5854: the desk transaction screen shows times in ICT and marks a reversal in the header.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/fe-triage-1101.md` W1 (PR #5848): desk times show in ICT, `dd/MM/yyyy HH:mm`
// (`desk-transaction-detail.md` D3 "no formatting" covers figures, not times). T5 (added to this ticket by
// the FE sub-controller, 2026-11-01): `reversal.md:55` (V-2) keeps the original COMPLETED on purpose, so
// the header says beside the status that the transaction was reversed, or that a reversal is pending.
// The sections are rendered on their own with the props `TransactionScreen.tsx:227-255` passes them.
// One test per row of the ticket's behaviour table; the row id is in the test name.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../shared/i18n';
import {
  makeReversalSummaryDto,
  makeTransactionAuditEventDto,
  makeTransactionDetailDto,
  makeTxPaymentDto,
  makeTxReceiptDto,
  type ReversalSummaryDto,
  type TransactionDetailDto,
} from '../../shared/testing/fixtures';
import { AuditSection } from './AuditSection';
import { HeaderSection } from './HeaderSection';
import { PaymentSection } from './PaymentSection';
import { ReceiptsSection } from './ReceiptsSection';
import { ReversalSection } from './ReversalSection';
import { th } from './i18n';

// The fixtures feed `…Z` and nanosecond `+00:00` times, as the server sends them.
const NANO = '2026-11-01T05:44:25.455952232+00:00'; // 01/11/2026 12:44
const LATE = '2026-10-31T17:30:00Z'; // 01/11/2026 00:30
const ISO = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/;

// T5 wording (ticket #5854 §7).
const T5 = {
  th: { reversed: 'กลับรายการแล้ว', pending: 'รอกลับรายการ' },
  en: { reversed: 'Reversed', pending: 'Reversal pending' },
} as const;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

function show(view: () => JSX.Element): HTMLDivElement {
  dispose = render(view, host);
  return host;
}

function text(el: Element | null | undefined = host): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

/** The `<dd>` beside the `<dt>` whose text is `label`. */
function valueOf(label: string): string {
  const dts = [...host.querySelectorAll('dt')].filter((d) => text(d) === label);
  expect(dts.length, `one field ${JSON.stringify(label)}`).toBe(1);
  return text(dts[0]!.nextElementSibling);
}

function tx(over: Partial<TransactionDetailDto> = {}) {
  return makeTransactionDetailDto({
    id: 'tx-1',
    status: 'COMPLETED',
    documentRef: 'XC-0001',
    branchId: 'MAIN',
    completedAt: NANO,
    cancelledAt: null,
    cancelReason: null,
    approvalRequest: null,
    reversal: null,
    ...over,
  });
}

function rev(status: string, over: Partial<ReversalSummaryDto> = {}) {
  return makeReversalSummaryDto({
    id: 'rv-1',
    originalTransactionId: 'tx-1',
    status,
    reason: 'ลูกค้าขอยกเลิก',
    amount: '35120.00',
    requestedBy: 'u-teller',
    requestedAt: LATE,
    approvedAt: status === 'REQUESTED' || status === 'REJECTED' ? null : '2026-11-01T04:10:00Z',
    postedAt: status === 'POSTED' ? NANO : null,
    documentNumber: status === 'POSTED' ? 'XC-0001-R2' : null,
    ...over,
  });
}

describe('FE-T6 #5854: times in ICT (design W1)', () => {
  it('H1: the header shows completedAt in ICT, not the ISO string', () => {
    show(() => <HeaderSection tx={tx()} />);
    expect(valueOf(th.completedAt)).toBe('01/11/2026 12:44');
    expect(text()).not.toMatch(ISO);
  });

  it('H2: a cancelled transaction shows cancelledAt in ICT, rolled to the Bangkok day', () => {
    show(() => <HeaderSection tx={tx({ status: 'CANCELLED', completedAt: null, cancelledAt: LATE, cancelReason: 'x' })} />);
    expect(valueOf(th.cancelledAt)).toBe('01/11/2026 00:30');
    expect(text()).not.toMatch(ISO);
  });

  it('P1: the payment shows paidAt in ICT', () => {
    show(() => (
      <PaymentSection
        payment={makeTxPaymentDto({
          method: 'CASH',
          reference: '',
          account: '',
          slipFileKey: null,
          amountThb: '35120.00',
          bankFeeThb: '0.00',
          paidAt: LATE,
        })}
      />
    ));
    expect(valueOf(th.paidAt)).toBe('01/11/2026 00:30');
    expect(valueOf(th.amountThb), 'figures stay as the server sent them (D3)').toBe('35120.00');
    expect(text()).not.toMatch(ISO);
  });

  it('RC1: each receipt row shows issuedAt in ICT', () => {
    show(() => (
      <ReceiptsSection
        receipts={[
          makeTxReceiptDto({ number: 'XC-0001', revision: 1, issuedAt: LATE }),
          makeTxReceiptDto({ number: 'XC-0001-R2', revision: 2, issuedAt: NANO }),
        ]}
        canReprint={false}
        busy={false}
        onReprint={() => undefined}
      />
    ));
    const t = text();
    expect(t).toContain('01/11/2026 00:30');
    expect(t).toContain('01/11/2026 12:44');
    expect(t).not.toMatch(ISO);
  });

  it('RV1: the reversal section shows requestedAt, approvedAt and postedAt in ICT', () => {
    show(() => <ReversalSection reversal={rev('POSTED')} />);
    expect(valueOf(th.requestedAt)).toBe('01/11/2026 00:30');
    expect(valueOf(th.approvedAt)).toBe('01/11/2026 11:10');
    expect(valueOf(th.postedAt)).toBe('01/11/2026 12:44');
    expect(valueOf(th.amount), 'figures stay as the server sent them (D3)').toBe('35120.00');
    expect(text()).not.toMatch(ISO);
  });

  it('RV2: a missing approvedAt/postedAt still hides its field', () => {
    show(() => <ReversalSection reversal={rev('REQUESTED')} />);
    expect([...host.querySelectorAll('dt')].map((d) => text(d))).not.toContain(th.approvedAt);
    expect([...host.querySelectorAll('dt')].map((d) => text(d))).not.toContain(th.postedAt);
  });

  it('A1: each audit row shows its time in ICT', () => {
    show(() => (
      <AuditSection
        events={[
          makeTransactionAuditEventDto({ at: LATE, actorId: 'u-mgr', actorRole: 'ROLE_A', action: 'approval.approved' }),
          makeTransactionAuditEventDto({ at: NANO, actorId: 'u-fin', actorRole: 'ROLE_B', action: 'reversal.approved' }),
        ]}
      />
    ));
    const t = text();
    expect(t).toContain('01/11/2026 00:30');
    expect(t).toContain('01/11/2026 12:44');
    expect(t).not.toMatch(ISO);
  });

  it('G1: a time that does not parse is shown as sent', () => {
    show(() => <HeaderSection tx={tx({ completedAt: 'not-a-time' })} />);
    expect(valueOf(th.completedAt)).toBe('not-a-time');
  });
});

describe('FE-T6 #5854 / T5: the header marks a reversal beside the status', () => {
  it('T5-1: COMPLETED with a POSTED reversal shows "COMPLETED · กลับรายการแล้ว <postedAt in ICT>"', () => {
    show(() => <HeaderSection tx={tx({ reversal: rev('POSTED') })} />);
    expect(valueOf(th.status)).toBe(`COMPLETED · ${T5.th.reversed} 01/11/2026 12:44`);
  });

  it('T5-1en: the same in English reads "COMPLETED · Reversed <postedAt in ICT>"', () => {
    setLang('en');
    show(() => <HeaderSection tx={tx({ reversal: rev('POSTED') })} />);
    const dt = [...host.querySelectorAll('dt')].find((d) => text(d) === 'Status');
    expect(dt, 'the en status label').toBeDefined();
    expect(text(dt!.nextElementSibling)).toBe(`COMPLETED · ${T5.en.reversed} 01/11/2026 12:44`);
  });

  it('T5-1b: a POSTED reversal without postedAt shows the mark without a time', () => {
    show(() => <HeaderSection tx={tx({ reversal: rev('POSTED', { postedAt: null }) })} />);
    expect(valueOf(th.status)).toBe(`COMPLETED · ${T5.th.reversed}`);
  });

  it('T5-2: a REQUESTED or APPROVED reversal shows that it is pending', () => {
    for (const s of ['REQUESTED', 'APPROVED']) {
      show(() => <HeaderSection tx={tx({ reversal: rev(s) })} />);
      expect(valueOf(th.status), s).toBe(`COMPLETED · ${T5.th.pending}`);
      dispose?.();
      dispose = undefined;
    }
  });

  it('T5-2en: pending in English reads "Reversal pending"', () => {
    setLang('en');
    show(() => <HeaderSection tx={tx({ reversal: rev('APPROVED') })} />);
    const dt = [...host.querySelectorAll('dt')].find((d) => text(d) === 'Status');
    expect(text(dt!.nextElementSibling)).toBe(`COMPLETED · ${T5.en.pending}`);
  });

  it('T5-3: no reversal, REJECTED or EXPIRED adds no text to the status', () => {
    for (const r of [null, rev('REJECTED'), rev('EXPIRED')]) {
      show(() => <HeaderSection tx={tx({ reversal: r })} />);
      expect(valueOf(th.status), String(r?.status ?? null)).toBe('COMPLETED');
      dispose?.();
      dispose = undefined;
    }
  });
});
