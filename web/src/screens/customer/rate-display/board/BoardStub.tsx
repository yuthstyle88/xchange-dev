// P01 v2 stub (rate-display.md §9.2, §9.6; B8): the conditions text, SYNCED (last successful update), SOURCE
// (the route's sourceDate), SNAPSHOT (the publication id), the opening hours (v1 D12) and the barcode.
import { Show, type JSX } from 'solid-js';
import { BOARD_TEXT, bilingual } from './board-texts';
import './board-stub.css';

export interface BoardStubProps {
  lastOkAt: number | undefined;
  sourceDate: string | null;
  snapshotId: string | null;
  /** The branch's opening hours from the branch list (v1 D12), or null. */
  hours: string | null;
}

function formatSynced(lastOkAt: number | undefined): string {
  if (lastOkAt === undefined || typeof lastOkAt !== 'number' || !Number.isFinite(lastOkAt)) {
    return '—';
  }
  try {
    return new Intl.DateTimeFormat('en-GB', {
      timeZone: 'Asia/Bangkok',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false,
    }).format(new Date(lastOkAt));
  } catch {
    return '—';
  }
}

export function BoardStub(props: BoardStubProps): JSX.Element {
  return (
    <footer class="xc-rb-stub">
      <div class="xc-rb-stub-legal">
        <strong class="xc-rb-stub-title">{BOARD_TEXT.stubTitle}</strong>
        <p class="xc-rb-stub-conditions">
          {BOARD_TEXT.stubConditions.map((cond, i) => (
            <>
              {i > 0 && <br />}
              {cond}
            </>
          ))}
        </p>
        <Show when={props.hours}>
          <p class="xc-rb-stub-hours">
            <small>{bilingual(BOARD_TEXT.hours)}</small> {props.hours}
          </p>
        </Show>
      </div>
      <div class="xc-rb-stub-meta">
        <small>{bilingual(BOARD_TEXT.synced_at)}</small>
        <span>{formatSynced(props.lastOkAt)}</span>
        <small>{bilingual(BOARD_TEXT.sourceDate)}</small>
        <span>{props.sourceDate || '—'}</span>
        <small>{bilingual(BOARD_TEXT.snapshot)}</small>
        <span>{props.snapshotId ? props.snapshotId.slice(0, 12) : '—'}</span>
      </div>
      <div class="xc-rb-barcode" aria-hidden="true" />
    </footer>
  );
}
