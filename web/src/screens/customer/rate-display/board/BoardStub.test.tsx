// RB-9 behaviour table unit tests (P01 v2 rate board; rate-display.md §9.2, §9.6; xchange#3570).
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { Ticker } from './Ticker';
import { BoardStub } from './BoardStub';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(ui: () => any) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(ui, host);
  return host;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('Ticker & BoardStub (RB-9)', () => {
  it('count 37: the count part is in the ticker, twice', () => {
    mount(() => <Ticker count={37} attract={true} />);
    const expected = '37 สกุลเงิน · 37 CURRENCIES · 37 种货币';
    const spans = Array.from(host.querySelectorAll('.xc-rb-ticker-item'));
    expect(spans.length).toBe(2);
    expect(spans[0].textContent).toContain(expected);
    expect(spans[1].textContent).toContain(expected);
    const countOccurrences = (host.textContent?.match(new RegExp(expected, 'g')) ?? []).length;
    expect(countOccurrences).toBe(2);
  });

  it('count 0: no count part', () => {
    mount(() => <Ticker count={0} attract={true} />);
    expect(host.querySelector('.xc-rb-ticker')).not.toBeNull();
    expect(host.textContent).toContain('ยินดีต้อนรับ');
    expect(host.textContent).toContain('WELCOME');
    expect(host.textContent).not.toContain('สกุลเงิน');
    expect(host.textContent).not.toContain('CURRENCIES');
    expect(host.textContent).not.toContain('种货币');
  });

  it('lastOkAt at 12:55:54Z: SYNCED 19:55:54', () => {
    const timestamp = new Date('2026-10-19T12:55:54Z').getTime();
    mount(() => (
      <BoardStub
        lastOkAt={timestamp}
        sourceDate="2026-10-19"
        snapshotId="1ecabdf3dffd"
        hours={null}
      />
    ));
    expect(host.textContent).toContain('SYNCED');
    expect(host.textContent).toContain('19:55:54');
  });

  it('snapshotId 1ecabdf3dffd4a…: SNAPSHOT 1ecabdf3dffd', () => {
    mount(() => (
      <BoardStub
        lastOkAt={undefined}
        sourceDate={null}
        snapshotId="1ecabdf3dffd4a5b6c"
        hours={null}
      />
    ));
    expect(host.textContent).toContain('SNAPSHOT');
    expect(host.textContent).toContain('1ecabdf3dffd');
    expect(host.textContent).not.toContain('1ecabdf3dffd4a');
  });

  it('sourceDate null: SOURCE —', () => {
    mount(() => (
      <BoardStub
        lastOkAt={undefined}
        sourceDate={null}
        snapshotId={null}
        hours={null}
      />
    ));
    expect(host.textContent).toContain('SOURCE');
    expect(host.textContent).toContain('—');
  });

  it("hours 09:00 - 18:00: shown; v1's RateDisplay.test.tsx hours test stays green", () => {
    mount(() => (
      <BoardStub
        lastOkAt={undefined}
        sourceDate={null}
        snapshotId={null}
        hours="09:00 - 18:00"
      />
    ));
    expect(host.textContent).toContain('09:00');
    expect(host.textContent).toContain('18:00');
    expect(host.textContent).toContain('OPEN');
  });
});
