// F20 #6430: desk detail shows the backdated mark when backdate is present
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeTransactionDetailDto } from '../../shared/testing/fixtures';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { HeaderSection } from './HeaderSection';

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

describe('HeaderSection backdated mark (#6430)', () => {
  it('E1: shows "ย้อนหลัง" and enteredAt when backdate is present', () => {
    const tx = makeTransactionDetailDto({
      completedAt: '2026-11-06T03:50:00Z',
      backdate: {
        enteredAt: '2026-11-06T03:58:00Z',
        enteredBy: 'u1',
      },
    });

    dispose = render(() => <HeaderSection tx={tx} />, host);

    expect(host.textContent).toContain('ย้อนหลัง');
    expect(host.textContent).toContain(`บันทึกเมื่อ ${formatBangkokDateTime('2026-11-06T03:58:00Z')}`);
  });

  it('E2: does not show "ย้อนหลัง" when backdate is absent or null', () => {
    const txNull = makeTransactionDetailDto({ backdate: null });
    dispose = render(() => <HeaderSection tx={txNull} />, host);
    expect(host.textContent).not.toContain('ย้อนหลัง');

    dispose();
    dispose = undefined;
    host.innerHTML = '';

    const txAbsent = makeTransactionDetailDto();
    delete txAbsent.backdate;
    dispose = render(() => <HeaderSection tx={txAbsent} />, host);
    expect(host.textContent).not.toContain('ย้อนหลัง');
  });
});
