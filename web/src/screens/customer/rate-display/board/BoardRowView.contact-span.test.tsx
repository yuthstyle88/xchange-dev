import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { BoardModel, BoardRow } from './board-model';
import { BoardRowView } from './BoardRowView';
import { BoardTable } from './BoardTable';

let host: HTMLElement;
let dispose: (() => void) | undefined;

function makeRow(over: Partial<BoardRow> = {}): BoardRow {
  return {
    key: 'KRW|more',
    code: 'KRW',
    group: null,
    unitLabel: '',
    cells: [],
    contactOnly: true,
    ...over,
  };
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BoardRowView contact cell colSpan (#5226)', () => {
  it('C1 a contact row, bandCount 3, not single column: td.xc-rb-contact-cell has colSpan 3', () => {
    host = document.createElement('tbody');
    document.body.appendChild(host);
    dispose = render(() => <BoardRowView row={makeRow()} singleColumn={false} bandCount={3} />, host);
    const cell = host.querySelector<HTMLTableCellElement>('td.xc-rb-contact-cell');
    expect(cell).not.toBeNull();
    expect(cell!.colSpan).toBe(3);
  });

  it('C2 the same row, single column: colSpan 1', () => {
    host = document.createElement('tbody');
    document.body.appendChild(host);
    dispose = render(() => <BoardRowView row={makeRow()} singleColumn={true} bandCount={3} />, host);
    const cell = host.querySelector<HTMLTableCellElement>('td.xc-rb-contact-cell');
    expect(cell).not.toBeNull();
    expect(cell!.colSpan).toBe(1);
  });

  it('C3 BoardTable with a model of 5 band headings, one contact row: the contact cell colSpan is 5', () => {
    host = document.createElement('div');
    document.body.appendChild(host);
    const model: BoardModel = {
      headings: [
        { tier: 'T1', label: 'T1' },
        { tier: 'T2', label: 'T2' },
        { tier: 'T3', label: 'T3' },
        { tier: 'T4', label: 'T4' },
        { tier: 'T5', label: 'T5' },
      ],
      singleColumn: false,
      rows: [makeRow()],
    };
    dispose = render(() => <BoardTable model={model} attract={false} />, host);
    const cell = host.querySelector<HTMLTableCellElement>('td.xc-rb-contact-cell');
    expect(cell).not.toBeNull();
    expect(cell!.colSpan).toBe(5);
  });
});
