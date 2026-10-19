// P01 v2 rates table (rate-display.md §9.2): headings in <thead>, one <tr> per row in <tbody>; sizes the rows to fit
// the canvas (--n, --fs) and runs the attract spotlight and wave when `attract` is on.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import { For, type JSX } from 'solid-js';
import type { BoardModel } from './board-model';
import { BandHeadings } from './BandHeadings';
import { BoardRowView } from './BoardRowView';
import './board-table.css';

export interface BoardTableProps {
  model: BoardModel;
  attract: boolean;
}

export function BoardTable(props: BoardTableProps): JSX.Element {
  return (
    <table class="xc-rb-table">
      <thead>
        <BandHeadings headings={props.model.headings} singleColumn={props.model.singleColumn} />
      </thead>
      <tbody>
        <For each={props.model.rows}>{(row) => <BoardRowView row={row} singleColumn={props.model.singleColumn} />}</For>
      </tbody>
    </table>
  );
}
