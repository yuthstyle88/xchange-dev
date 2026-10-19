// P01 v2 header (rate-display.md §9.2): the brand and the clock.
import type { JSX } from 'solid-js';
import { BOARD_TEXT } from './board-texts';
import { BoardClock } from './BoardClock';
import './board-header.css';

export interface BoardHeaderProps {
  now?: () => Date;
}

export function BoardHeader(props: BoardHeaderProps = {}): JSX.Element {
  return (
    <header class="xc-rb-head">
      <i class="xc-rb-sheen" aria-hidden="true" />
      <div class="xc-rb-brand">
        <strong>{BOARD_TEXT.brand}</strong>
        <small>{BOARD_TEXT.brandSub}</small>
      </div>
      <BoardClock now={props.now} />
    </header>
  );
}
