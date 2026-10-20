import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeShortReceiptRow,
  makeTransferStatusCount,
  makeTransferSummary,
  type TransferSummary,
} from '../../../../shared/testing/fixtures';
import { setLang } from '../../../../shared/i18n';
import { TransfersPart } from './TransfersPart';
import * as table from './i18n';

function summary(over: Partial<TransferSummary> = {}) {
  return makeTransferSummary({
    byStatus: [],
    inTransit: [],
    shortReceipts: [],
    ...over,
  });
}

describe('A5: F01 transfers part reconcile link (#3331)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    setLang('th');
  });

  function mount(data: TransferSummary) {
    dispose = render(() => <TransfersPart data={data} />, root);
  }

  it('row 1: byStatus has {status: "RECEIVED", count: 2} -> one a[data-action="reconcile"] with href="#cash"', () => {
    mount(summary({ byStatus: [makeTransferStatusCount({ status: 'RECEIVED', count: 2 })] }));
    const links = root.querySelectorAll<HTMLAnchorElement>('a[data-action="reconcile"]');
    expect(links).toHaveLength(1);
    expect(links[0]!.getAttribute('href')).toBe('#cash');
  });

  it('row 2: no RECEIVED (or count 0) and shortReceipts empty -> no reconcile link', () => {
    mount(
      summary({
        byStatus: [
          makeTransferStatusCount({ status: 'SENT', count: 3 }),
          makeTransferStatusCount({ status: 'RECEIVED', count: 0 }),
        ],
        shortReceipts: [],
      }),
    );
    expect(root.querySelector('a[data-action="reconcile"]')).toBeNull();
  });

  it('row 3: no RECEIVED but one short receipt -> the reconcile link shows', () => {
    mount(
      summary({
        byStatus: [makeTransferStatusCount({ status: 'RECONCILED', count: 1 })],
        shortReceipts: [makeShortReceiptRow({ transferId: 'TR-1', currency: 'USD', sent: '1000', received: '990' })],
      }),
    );
    const link = root.querySelector<HTMLAnchorElement>('a[data-action="reconcile"]');
    expect(link).not.toBeNull();
    expect(link!.getAttribute('href')).toBe('#cash');
  });

  it('row 4: a short receipt sent 1000 received 990 shows its transfer id, currency, sent and received', () => {
    mount(
      summary({
        shortReceipts: [makeShortReceiptRow({ transferId: 'TR-99', currency: 'USD', sent: '1000', received: '990' })],
      }),
    );
    const text = root.textContent ?? '';
    expect(text).toContain('TR-99');
    expect(text).toContain('USD');
    expect(text).toContain('1000');
    expect(text).toContain('990');
  });

  it('row 5: Thai / English link text matches the i18n table and is never a raw key', () => {
    setLang('th');
    mount(summary({ byStatus: [{ status: 'RECEIVED', count: 1 }] }));
    const link = root.querySelector<HTMLAnchorElement>('a[data-action="reconcile"]')!;
    expect(link.textContent?.trim()).toBe(table.th.reconcileInCash);
    expect(link.textContent?.trim()).toBe('กระทบยอดที่เมนูเงินสด');
    expect(link.textContent).not.toContain('reconcileInCash');

    setLang('en');
    expect(link.textContent?.trim()).toBe(table.en.reconcileInCash);
    expect(link.textContent?.trim()).toBe('Reconcile in Cash');
    expect(link.textContent).not.toContain('reconcileInCash');
  });
});
