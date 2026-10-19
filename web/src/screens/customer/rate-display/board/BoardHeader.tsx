// P01 v2 header (rate-display.md §9.2): the brand and the clock.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import { BOARD_TEXT } from './board-texts';
import { BoardClock } from './BoardClock';
import './board-header.css';

export function BoardHeader(): JSX.Element {
  return (
    <header class="xc-rb-head">
      <strong>{BOARD_TEXT.brand}</strong>
      <BoardClock />
    </header>
  );
}
