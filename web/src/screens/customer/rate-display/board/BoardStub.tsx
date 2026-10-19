// P01 v2 stub (rate-display.md §9.2, §9.6; B8): the conditions text, SYNCED (last successful update), SOURCE
// (the route's sourceDate), SNAPSHOT (the publication id), the opening hours (v1 D12) and the barcode.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import './board-stub.css';

export interface BoardStubProps {
  lastOkAt: number | undefined;
  sourceDate: string | null;
  snapshotId: string | null;
  /** The branch's opening hours from the branch list (v1 D12), or null. */
  hours: string | null;
}

export function BoardStub(props: BoardStubProps): JSX.Element {
  return (
    <footer class="xc-rb-stub">
      <p>{props.hours}</p>
    </footer>
  );
}
