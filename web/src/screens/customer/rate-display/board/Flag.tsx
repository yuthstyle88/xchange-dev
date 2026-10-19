// P01 v2 flag (rate-display.md §9.7): the vendored flag image, or a code box when there is none or it fails to load.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import './flag.css';

export interface FlagProps {
  currency: string;
}

export function Flag(props: FlagProps): JSX.Element {
  return <span class="xc-rb-code-box">{props.currency}</span>;
}
