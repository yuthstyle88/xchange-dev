// G3-FE #5918: the audit row names the actor, not the raw id (desk-transaction-detail.md D7, amended 2026-11-02).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// D7: `actorName` is the actor's `users.name`, or null when the actor is not a user (SYSTEM, CUSTOMER,
// POLICY, legacy ids) or the name is blank. The fallback agreed with the BE sub-controller: the role word,
// with the id only in the cell's `title`. The role word is AccountBar's `shell.account.role.<camel>` label;
// a role code with no label shows as itself. One test per row of the ticket's behaviour table.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { th as shellTh } from '../../shared/i18n/th';
import { en as shellEn } from '../../shared/i18n/en';
import { makeTransactionAuditEventDto, type TransactionAuditEventDto } from '../../shared/testing/fixtures';
import { AuditSection } from './AuditSection';

const ID = '4c522b2d-6f1e-4a0b-9d3c-2a7e5b1c0f11';

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

function show(events: TransactionAuditEventDto[]): void {
  dispose = render(() => <AuditSection events={events} />, host);
}

/** The actor cell (second column) of each body row. */
function actorCells(): HTMLTableCellElement[] {
  return [...host.querySelectorAll('tbody tr')].map((r) => r.querySelectorAll('td')[1] as HTMLTableCellElement);
}

function text(el: Element): string {
  return (el.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('G3-FE the audit actor cell (D7)', () => {
  it('N1: a named actor shows "name · role word"; the id is only in title', () => {
    show([makeTransactionAuditEventDto({ actorId: ID, actorName: 'Somchai', actorRole: 'BRANCH_MANAGER' })]);
    const [cell] = actorCells();
    expect(text(cell!)).toBe(`Somchai · ${shellTh['shell.account.role.branchManager']}`);
    expect(host.textContent).not.toContain(ID);
    expect(cell!.getAttribute('title')).toBe(ID);
  });

  it('N2: actorName null shows the role word alone; the id is only in title', () => {
    show([makeTransactionAuditEventDto({ actorId: ID, actorName: null, actorRole: 'TELLER' })]);
    const [cell] = actorCells();
    expect(text(cell!)).toBe(shellTh['shell.account.role.teller']);
    expect(host.textContent).not.toContain(ID);
    expect(cell!.getAttribute('title')).toBe(ID);
  });

  it('N3: actorName null and a role with no label (SYSTEM) shows the role code', () => {
    show([makeTransactionAuditEventDto({ actorId: 'SYSTEM', actorName: null, actorRole: 'SYSTEM' })]);
    const [cell] = actorCells();
    expect(text(cell!)).toBe('SYSTEM');
    expect(cell!.getAttribute('title')).toBe('SYSTEM');
  });

  it('N4: the role word follows the language', () => {
    setLang('en');
    show([makeTransactionAuditEventDto({ actorId: ID, actorName: 'Somchai', actorRole: 'FINANCE' })]);
    expect(text(actorCells()[0]!)).toBe(`Somchai · ${shellEn['shell.account.role.finance']}`);
  });

  it('N5: each row names its own actor, in the order sent', () => {
    show([
      makeTransactionAuditEventDto({ actorId: 'u-mgr', actorName: 'Somchai', actorRole: 'BRANCH_MANAGER', action: 'approval.approved' }),
      makeTransactionAuditEventDto({ actorId: 'u-sys', actorName: null, actorRole: 'SYSTEM', action: 'reversal.approved' }),
    ]);
    const cells = actorCells().map(text);
    expect(cells).toEqual([`Somchai · ${shellTh['shell.account.role.branchManager']}`, 'SYSTEM']);
    expect(host.textContent).not.toContain('u-mgr');
  });
});
