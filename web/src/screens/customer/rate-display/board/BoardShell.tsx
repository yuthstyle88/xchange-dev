// P01 v2 board shell (rate-display.md §9.1, §9.2): the 1080×1920 portrait canvas scaled to the screen, the calm
// switch (?attract=0), and the composition of the board's parts. The composition is the skeleton's; RB-4 adds the
// scaling (--scale = min(innerWidth/1080, innerHeight/1920), updated on resize) inside this file.
import { Show, type JSX } from 'solid-js';
import type { BoardModel } from './board-model';
import type { BoardStatusView } from './board-status';
import { BookingQr } from '../BookingQr';
import { BoardHeader } from './BoardHeader';
import { BoardStub } from './BoardStub';
import { BoardTable } from './BoardTable';
import { RouteStrip } from './RouteStrip';
import { Ticker } from './Ticker';
import './board-shell.css';

export interface BoardShellProps {
  branch: string;
  branchName: string;
  model: BoardModel;
  status: BoardStatusView;
  stale: boolean;
  lastOkAt: number | undefined;
  sourceDate: string | null;
  snapshotId: string | null;
  hours: string | null;
  /** The branch list's `bookingEnabled` for this branch: the booking QR (v1 D9/D9a, RD-4 merged) shows only when true. */
  bookingEnabled: boolean | undefined;
  /** False with `?attract=0`: no spotlight, wave, ticker motion or blink. */
  attract: boolean;
}

export function BoardShell(props: BoardShellProps): JSX.Element {
  return (
    <div
      class="xc-rb"
      classList={{
        'xc-rb-calm': !props.attract,
        'xc-rb-offline': props.status.status === 'offline',
        'xc-rb-delayed': props.status.status === 'delayed',
      }}
    >
      <BoardHeader />
      <RouteStrip
        count={props.model.rows.length}
        branch={props.branch}
        branchName={props.branchName}
        status={props.status}
        stale={props.stale}
      />
      <Show when={props.status.ratesVisible && props.model.rows.length > 0}>
        <BoardTable model={props.model} attract={props.attract} />
      </Show>
      <Show when={props.status.emptyMessage}>
        {(message) => <div class="xc-rb-empty">{message()}</div>}
      </Show>
      <Ticker count={props.model.rows.length} attract={props.attract} />
      <BoardStub lastOkAt={props.lastOkAt} sourceDate={props.sourceDate} snapshotId={props.snapshotId} hours={props.hours} />
      <BookingQr branchCode={props.branch} bookingEnabled={props.bookingEnabled} />
    </div>
  );
}
