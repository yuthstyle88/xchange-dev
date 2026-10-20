// P01 v2 one currency row (rate-display.md §9.2, §9.4): flag, code, group, unit, one cell (buy · sell) per band.
import { createMemo, Index, Show, type JSX } from 'solid-js';
import type { BoardRow } from './board-model';
import { compareDecimal, formatBandRate } from './decimal-format';
import { Flag } from './Flag';
import { FlapPrice } from './FlapPrice';
import './board-table.css';

export interface BoardRowViewProps {
  row: BoardRow;
  singleColumn: boolean;
}

function createSideState(raw: () => string | null) {
  const rawMemo = createMemo(raw);
  let previousRaw: string | null | undefined = undefined;
  return createMemo(() => {
    const current = rawMemo();
    let direction: -1 | 0 | 1 = 0;
    if (previousRaw !== undefined && previousRaw !== null && current !== null) {
      direction = compareDecimal(current, previousRaw);
    }
    previousRaw = current;
    return {
      text: formatBandRate(current),
      direction,
    };
  });
}

export function BoardRowView(props: BoardRowViewProps): JSX.Element {
  return (
    <tr class="xc-rb-row">
      <th scope="row" class="xc-rb-cur">
        <Flag currency={props.row.code} />
        <span class="xc-rb-cur-text">
          <b class="xc-rb-cur-code">{props.row.code}</b>
          <Show when={props.row.group}>{(group) => <span class="xc-rb-cur-group">{group()}</span>}</Show>
          <Show when={props.row.unitLabel}>
            <span class="xc-rb-cur-unit">{props.row.unitLabel}</span>
          </Show>
        </span>
      </th>
      <Index each={props.row.cells}>
        {(cell) => {
          const buy = createSideState(() => cell().buy);
          const sell = createSideState(() => cell().sell);
          return (
            <td class="xc-rb-band-cell" classList={{ 'xc-rb-band-repeat': cell().repeat }}>
              <span class="xc-rb-side xc-rb-buy">
                <FlapPrice text={buy().text} direction={buy().direction} />
              </span>
              <span class="xc-rb-side xc-rb-sell">
                <FlapPrice text={sell().text} direction={sell().direction} />
              </span>
            </td>
          );
        }}
      </Index>
    </tr>
  );
}
