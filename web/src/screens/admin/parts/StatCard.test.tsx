// V2-parts (#476): the stat card (ticket §7 `StatCard.tsx`, design §12.2, behaviour P1, P6).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// v2 `admin-html:61`: label, the figure with its unit beside it (`THB` unless given), and a note.
// v3: `value` is the server's decimal string, shown through `MoneyText` (W7) — so a figure a
// JavaScript number cannot hold keeps every digit.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { StatCard } from './StatCard';

const BIG = '90071992547409.93';

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
});

describe('StatCard (#476 P1, P6)', () => {
  it('shows the label, the server figure through MoneyText, THB by default, and the note', () => {
    const el = mount(() => <StatCard label="ยอดรับซื้อ" value={BIG} note="ตามช่วงที่เลือก" />);
    const text = el.textContent ?? '';
    expect(text).toContain('ยอดรับซื้อ');
    expect(formatMoney(BIG)).toBe('90,071,992,547,409.93');
    expect(text).toContain('90,071,992,547,409.93');
    expect(text).toContain('THB');
    expect(text).toContain('ตามช่วงที่เลือก');
    expect(text.indexOf('ยอดรับซื้อ')).toBeLessThan(text.indexOf('90,071,992,547,409.93'));
    expect(text.indexOf('90,071,992,547,409.93')).toBeLessThan(text.indexOf('ตามช่วงที่เลือก'));
  });

  it('shows the unit it is given instead of THB', () => {
    const el = mount(() => <StatCard label="USD" value="1250.50" note="n" unit="รายการ" />);
    const text = el.textContent ?? '';
    expect(text).toContain('1,250.50');
    expect(text).toContain('รายการ');
    expect(text).not.toContain('THB');
  });

  it("keeps the server's decimals as they are: no rounding, no padding", () => {
    const el = mount(() => <StatCard label="a" value="0.125" note="b" />);
    expect(el.textContent).toContain('0.125');
    dispose?.();
    root.remove();
    const el2 = mount(() => <StatCard label="a" value="-7" note="b" />);
    expect(el2.textContent).toContain('-7');
    expect(el2.textContent).not.toContain('-7.00');
  });
});
