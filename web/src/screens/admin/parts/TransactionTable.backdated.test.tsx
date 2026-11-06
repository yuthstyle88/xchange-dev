// F20 #6430: report table shows "ย้อนหลัง" pill for backdated rows
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { TransactionTable } from './TransactionTable';

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

describe('TransactionTable backdated mark (#6430)', () => {
  it('E3: displays [data-backdated="true"] with "ย้อนหลัง" on backdated rows only', () => {
    const r1 = makeTransactionRow({ id: 'TX-1', backdated: true, status: 'COMPLETED' });
    const r2 = makeTransactionRow({ id: 'TX-2', backdated: false, status: 'COMPLETED' });

    dispose = render(() => <TransactionTable rows={[r1, r2]} onOpen={() => {}} />, host);

    const rows = host.querySelectorAll('tbody tr');
    expect(rows.length).toBe(2);

    const cell1 = rows[0].querySelector('td:last-child');
    const pill1 = cell1?.querySelector('[data-backdated="true"]');
    expect(pill1).not.toBeNull();
    expect(pill1?.textContent).toBe('ย้อนหลัง');
    expect(pill1?.classList.contains('pill')).toBe(true);

    const cell2 = rows[1].querySelector('td:last-child');
    const pill2 = cell2?.querySelector('[data-backdated="true"]');
    expect(pill2).toBeNull();
  });

  it('E6: keeps existing status pill text and cancel class unchanged', () => {
    const r = makeTransactionRow({ id: 'TX-3', backdated: true, status: 'CANCELLED' });

    dispose = render(() => <TransactionTable rows={[r]} onOpen={() => {}} />, host);

    const cell = host.querySelector('tbody tr td:last-child');
    const statusPill = cell?.querySelector('.pill:not([data-backdated])');
    expect(statusPill?.textContent).toBe('ยกเลิก');
    expect(statusPill?.classList.contains('cancel')).toBe(true);

    const backdatedPill = cell?.querySelector('[data-backdated="true"]');
    expect(backdatedPill?.textContent).toBe('ย้อนหลัง');
  });
});
