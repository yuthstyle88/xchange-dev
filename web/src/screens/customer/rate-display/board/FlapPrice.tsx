// P01 v2 split-flap price cell (rate-display.md §9.2, §9.6): one span per character; only the characters that
// changed since the previous text flip; an arrow and an up/down glow when `direction` is not 0.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import './flap-price.css';

export interface FlapPriceProps {
  /** The formatted text (formatBandRate), e.g. `35.42` or `—`. */
  text: string;
  /** 1 when the rate rose since the previous text, -1 when it fell, 0 for no arrow (computed by the row). */
  direction: -1 | 0 | 1;
}

export function FlapPrice(props: FlapPriceProps): JSX.Element {
  return <span class="xc-rb-price">{props.text}</span>;
}
