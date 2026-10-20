// P01 v2 route strip (rate-display.md §9.2, §9.5): FROM THB · TO <count> currencies · BRANCH code + name ·
// STATUS (synced / delayed / offline) and, when the route says so, the stale-reference line.
import { Show, type JSX } from 'solid-js';
import type { BoardStatusView } from './board-status';
import { BOARD_TEXT, bilingual } from './board-texts';
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
  const statusText = () => {
    const s = props.status?.status;
    if (s === 'delayed') return BOARD_TEXT.delayed;
    if (s === 'offline') return BOARD_TEXT.offline;
    return BOARD_TEXT.synced;
  };

  return (
    <section class="xc-rb-route">
      <div class="xc-rb-route-grid">
        <div class="xc-rb-route-from">
          <small class="xc-rb-route-label">{bilingual(BOARD_TEXT.from)}</small>
          <b class="xc-rb-route-code">{BOARD_TEXT.fromCurrency.code}</b>
          <span class="xc-rb-route-name">{BOARD_TEXT.fromCurrency.name}</span>
        </div>
        <div class="xc-rb-route-plane" aria-hidden="true">
          <i class="xc-rb-route-track" />
          <span class="xc-rb-route-jet">✈</span>
        </div>
        <div class="xc-rb-route-to">
          <small class="xc-rb-route-label">{bilingual(BOARD_TEXT.to)}</small>
          <b class="xc-rb-route-count">{props.count}</b>
          <span class="xc-rb-route-currencies">{bilingual(BOARD_TEXT.currencies)}</span>
        </div>
        <div class="xc-rb-route-branch">
          <small class="xc-rb-route-label">{bilingual(BOARD_TEXT.branch)}</small>
          <b class="xc-rb-route-branch-code">{props.branch}</b>
          <span class="xc-rb-route-branch-name">{props.branchName}</span>
        </div>
        <div class="xc-rb-route-state">
          <small class="xc-rb-route-label">
            <i class="xc-rb-live-dot" aria-hidden="true" />
            {bilingual(BOARD_TEXT.status)}
          </small>
          <b class="xc-rb-route-status" role="status">
            {statusText().th}
          </b>
          <span class="xc-rb-route-substatus">
            {statusText().en}
          </span>
        </div>
      </div>
      <Show when={props.stale}>
        <div class="xc-rb-route-stale">{bilingual(BOARD_TEXT.staleReference)}</div>
      </Show>
    </section>
  );
}
