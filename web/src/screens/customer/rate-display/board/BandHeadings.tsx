// P01 v2 band headings (rate-display.md §9.4): the currency heading, then one heading per band (or "all amounts").
import { For, Show, type JSX } from 'solid-js';
import type { BandHeading } from './board-model';
import { BOARD_TEXT, bilingual } from './board-texts';
import './board-table.css';

export interface BandHeadingsProps {
  headings: BandHeading[];
  singleColumn: boolean;
}

export function BandHeadings(props: BandHeadingsProps): JSX.Element {
  return (
    <tr class="xc-rb-heading-row">
      <th scope="col" class="xc-rb-cur-h">
        <span class="xc-rb-h-main">{BOARD_TEXT.currencyHeading.th}</span>
        <small class="xc-rb-h-sub">{BOARD_TEXT.currencyHeading.en}</small>
      </th>
      <Show
        when={!props.singleColumn}
        fallback={
          <th scope="col" class="xc-rb-band-h">
            <span class="xc-rb-h-main">{bilingual(BOARD_TEXT.allAmounts)}</span>
            <small class="xc-rb-h-sub">{bilingual(BOARD_TEXT.bandSub)}</small>
          </th>
        }
      >
        <For each={props.headings}>
          {(h) => (
            <th scope="col" class="xc-rb-band-h">
              <span class="xc-rb-h-main">{h.label}</span>
              <small class="xc-rb-h-sub">{bilingual(BOARD_TEXT.bandSub)}</small>
            </th>
          )}
        </For>
      </Show>
    </tr>
  );
}
