import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { BoardShell, type BoardShellProps } from './BoardShell';
import type { BoardModel } from './board-model';

function mockModel(): BoardModel {
  return {
    headings: [{ tier: 'T1', label: 'T1' }],
    singleColumn: false,
    rows: [
      {
        key: 'USD',
        code: 'USD',
        group: null,
        unitLabel: '',
        cells: [{ tier: 'T1', buy: '33.00', sell: '34.00', repeat: false }],
      },
    ],
  };
}

function mockProps(overrides?: Partial<BoardShellProps>): BoardShellProps {
  return {
    branch: 'BKK',
    branchName: 'Bangkok Branch',
    model: mockModel(),
    status: { status: 'synced', ratesVisible: true, emptyMessage: null },
    stale: false,
    lastOkAt: 1700000000000,
    sourceDate: '2026-10-19',
    snapshotId: 'snap-1',
    hours: '09:00 - 18:00',
    bookingEnabled: true,
    attract: true,
    ...overrides,
  };
}

function setWindowSize(width: number, height: number): void {
  Object.defineProperty(window, 'innerWidth', { writable: true, configurable: true, value: width });
  Object.defineProperty(window, 'innerHeight', { writable: true, configurable: true, value: height });
  window.dispatchEvent(new Event('resize'));
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

function mount(props: BoardShellProps): { el: HTMLDivElement; dispose: () => void } {
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <BoardShell {...props} />, container);
  return { el: container, dispose };
}

beforeEach(() => {
  document.body.innerHTML = '';
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('RB-4 BoardShell: 1080x1920 canvas scaling, calm mode, states', () => {
  it('mount at 540x960: --scale is 0.5', () => {
    setWindowSize(540, 960);
    const { el } = mount(mockProps());
    const root = el.querySelector<HTMLElement>('.xc-rb');
    expect(root?.style.getPropertyValue('--scale')).toBe('0.5');
  });

  it('resize to 1080x1920: --scale is 1', () => {
    setWindowSize(540, 960);
    const { el } = mount(mockProps());
    const root = el.querySelector<HTMLElement>('.xc-rb');
    expect(root?.style.getPropertyValue('--scale')).toBe('0.5');

    setWindowSize(1080, 1920);
    expect(root?.style.getPropertyValue('--scale')).toBe('1');
  });

  it('attract=false: .xc-rb-calm on the root', () => {
    const { el } = mount(mockProps({ attract: false }));
    const root = el.querySelector<HTMLElement>('.xc-rb');
    expect(root?.classList.contains('xc-rb-calm')).toBe(true);
  });

  it('status offline: .xc-rb-offline; the table is not rendered', () => {
    const { el } = mount(
      mockProps({
        status: { status: 'offline', ratesVisible: true, emptyMessage: null },
      }),
    );
    const root = el.querySelector<HTMLElement>('.xc-rb');
    expect(root?.classList.contains('xc-rb-offline')).toBe(true);
    expect(el.querySelector('.xc-rb-table')).toBeNull();
  });

  it('status delayed: .xc-rb-delayed', () => {
    const { el } = mount(
      mockProps({
        status: { status: 'delayed', ratesVisible: true, emptyMessage: null },
      }),
    );
    const root = el.querySelector<HTMLElement>('.xc-rb');
    expect(root?.classList.contains('xc-rb-delayed')).toBe(true);
  });

  it('unmount: the resize listener is removed', () => {
    const addSpy = vi.spyOn(window, 'addEventListener');
    const removeSpy = vi.spyOn(window, 'removeEventListener');
    const { dispose } = mount(mockProps());
    expect(addSpy).toHaveBeenCalledWith('resize', expect.any(Function));
    const listener = addSpy.mock.calls.find((call) => call[0] === 'resize')?.[1];
    dispose();
    expect(removeSpy).toHaveBeenCalledWith('resize', listener);
  });
});
