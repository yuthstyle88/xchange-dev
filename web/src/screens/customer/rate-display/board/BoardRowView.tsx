// P01 v2 one currency row (rate-display.md §9.2, §9.4): flag, code, group, unit, one cell (buy · sell) per band.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { BoardRow } from './board-model';
import './board-table.css';

export interface BoardRowViewProps {
  row: BoardRow;
  singleColumn: boolean;
}

export function BoardRowView(props: BoardRowViewProps): JSX.Element {
  return (
    <tr class="xc-rb-row">
      <th scope="row">{props.row.code}</th>
    </tr>
  );
}
