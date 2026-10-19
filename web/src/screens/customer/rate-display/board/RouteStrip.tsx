// P01 v2 route strip (rate-display.md §9.2, §9.5): FROM THB · TO <count> currencies · BRANCH code + name ·
// STATUS (synced / delayed / offline) and, when the route says so, the stale-reference line.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { BoardStatusView } from './board-status';
import './route-strip.css';

export interface RouteStripProps {
  count: number;
  branch: string;
  branchName: string;
  status: BoardStatusView;
  /** The route's `stale` (D5): shown as its own line; never hides rates. */
  stale: boolean;
}

export function RouteStrip(props: RouteStripProps): JSX.Element {
  return <section class="xc-rb-route">{props.branchName}</section>;
}
