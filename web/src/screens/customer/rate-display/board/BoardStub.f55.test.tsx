// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// Do not edit, delete or weaken these tests.
//
// SOURCE is the route's sourceDate, a day sent as YYYY-MM-DD: it is shown with formatBangkokDate (dd/mm/yyyy),
// never as the raw ISO day; a missing sourceDate still shows "—".
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { BoardStub } from './BoardStub';

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
});

function metaSpans(sourceDate: string | null): string[] {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <BoardStub lastOkAt={undefined} sourceDate={sourceDate} snapshotId={null} hours={null} />,
    host,
  );
  return Array.from(host.querySelectorAll('.xc-rb-stub-meta > span')).map((e) => e.textContent ?? '');
}

describe('F55 BoardStub SOURCE date', () => {
  it("sourceDate '2026-11-04': SOURCE reads 04/11/2026, not 2026-11-04", () => {
    // spans: SYNCED, SOURCE, SNAPSHOT
    expect(metaSpans('2026-11-04')).toEqual(['—', '04/11/2026', '—']);
    expect(host!.textContent).not.toContain('2026-11-04');
  });

  it('sourceDate null: SOURCE stays —', () => {
    expect(metaSpans(null)).toEqual(['—', '—', '—']);
  });
});
