// RD-3 rate board component (design rate-display.md §3, D6).
import { For, Show, type JSX } from 'solid-js';
import type { BoardRow } from './board-rows';

export interface RateBoardProps {
  rows: BoardRow[];
  currencyHeader: string;
  unitHeader: string;
  weBuyHeader: string;
  weSellHeader: string;
  weBuyHint: string;
  weSellHint: string;
}

export function RateBoard(props: RateBoardProps): JSX.Element {
  return (
    <div class="xc-rate-board-container">
      <table class="xc-rate-board-table">
        <thead>
          <tr class="xc-rate-board-head-row">
            <th class="xc-rate-board-th-currency">{props.currencyHeader}</th>
            <th class="xc-rate-board-th-unit">{props.unitHeader}</th>
            <th class="xc-rate-board-th-buy">
              <div class="xc-rate-board-col-title">{props.weBuyHeader}</div>
              <div class="xc-rate-board-col-hint">{props.weBuyHint}</div>
            </th>
            <th class="xc-rate-board-th-sell">
              <div class="xc-rate-board-col-title">{props.weSellHeader}</div>
              <div class="xc-rate-board-col-hint">{props.weSellHint}</div>
            </th>
          </tr>
        </thead>
        <tbody>
          <For each={props.rows}>
            {(row) => (
              <tr class="xc-rate-board-row">
                <td class="xc-rate-board-td-currency">
                  <div class="xc-rate-board-currency-code">{row.currency}</div>
                  <Show when={row.group}>
                    <div class="xc-rate-board-group-badge">{row.group}</div>
                  </Show>
                </td>
                <td class="xc-rate-board-td-unit">{row.unitLabel}</td>
                <td class="xc-rate-board-td-buy">{row.buy}</td>
                <td class="xc-rate-board-td-sell">{row.sell}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
