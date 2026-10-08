// V2-parts (#476): the table with its empty state (ticket §7 `DataTable.tsx`, design §12.2, P1).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// v2 `admin-html:62`: no rows → no table, the empty state `ไม่พบรายการในช่วงที่เลือก` with the note
// `เปลี่ยนช่วงวันที่หรือตัวกรอง แล้วลองอีกครั้ง` (the first replaceable by the caller); rows → one table,
// the headers in order, a numeric header (v2's `~` prefix) carrying the class `num`, the rows in
// the body.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { DataTable } from './DataTable';

const THAI = /[฀-๿]/;
const HEADERS = [{ text: 'วัน' }, { text: 'ยอด', numeric: true }, { text: 'สถานะ' }];

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
  setLang('th');
});

describe('DataTable (#476 P1)', () => {
  it("no rows: v2's empty state and no table, and the children are not drawn", () => {
    const el = mount(() => (
      <DataTable headers={HEADERS} rowCount={0}>
        <tr>
          <td>should-not-show</td>
        </tr>
      </DataTable>
    ));
    expect(el.textContent).toContain('ไม่พบรายการในช่วงที่เลือก');
    expect(el.textContent).toContain('เปลี่ยนช่วงวันที่หรือตัวกรอง แล้วลองอีกครั้ง');
    expect(el.querySelector('table')).toBeNull();
    expect(el.textContent).not.toContain('should-not-show');
  });

  it('no rows: the empty texts the caller gives replace the defaults', () => {
    const el = mount(() => (
      <DataTable headers={HEADERS} rowCount={0} empty="ยังไม่มีสลิป" emptyNote="แนบสลิปก่อน">
        <tr />
      </DataTable>
    ));
    expect(el.textContent).toContain('ยังไม่มีสลิป');
    expect(el.textContent).toContain('แนบสลิปก่อน');
    expect(el.textContent).not.toContain('ไม่พบรายการในช่วงที่เลือก');
    expect(el.textContent).not.toContain('เปลี่ยนช่วงวันที่หรือตัวกรอง แล้วลองอีกครั้ง');
  });

  it('rows: one table, the headers in order, numeric ones with the class num, the rows in its body', () => {
    const el = mount(() => (
      <DataTable headers={HEADERS} rowCount={2}>
        <tr>
          <td>r1</td>
        </tr>
        <tr>
          <td>r2</td>
        </tr>
      </DataTable>
    ));
    const tables = el.querySelectorAll('table');
    expect(tables.length).toBe(1);
    const ths = [...el.querySelectorAll('thead th')];
    expect(ths.map((th) => th.textContent)).toEqual(['วัน', 'ยอด', 'สถานะ']);
    expect(ths.map((th) => th.classList.contains('num'))).toEqual([false, true, false]);
    const cells = [...el.querySelectorAll('tbody td')].map((td) => td.textContent);
    expect(cells).toEqual(['r1', 'r2']);
    expect(el.textContent).not.toContain('ไม่พบรายการในช่วงที่เลือก');
  });

  it('the empty state follows rowCount as it changes', () => {
    const [count, setCount] = createSignal(0);
    const el = mount(() => (
      <DataTable headers={HEADERS} rowCount={count()}>
        <tr>
          <td>row</td>
        </tr>
      </DataTable>
    ));
    expect(el.querySelector('table')).toBeNull();
    setCount(1);
    expect(el.querySelector('table')).not.toBeNull();
    expect(el.textContent).not.toContain('ไม่พบรายการในช่วงที่เลือก');
    setCount(0);
    expect(el.querySelector('table')).toBeNull();
    expect(el.textContent).toContain('ไม่พบรายการในช่วงที่เลือก');
  });

  it('in English the default empty texts switch (W10)', () => {
    setLang('en');
    const el = mount(() => (
      <DataTable headers={[{ text: 'Date' }]} rowCount={0}>
        <tr />
      </DataTable>
    ));
    expect((el.textContent ?? '').trim().length).toBeGreaterThan(0);
    expect(THAI.test(el.textContent ?? '')).toBe(false);
  });
});
