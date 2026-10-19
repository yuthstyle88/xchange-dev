// B06 board tests (FE-desk, #564, #3336). Tests every row of the behaviour table.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { Board } from './Board';
import { th } from './i18n';
import type { QueueBoardData } from '../../../../shared/api/generated/queue_board';
import type { QueueError, QueueStore } from '../../../../shared/queue/queue-store';

const mountedActions = vi.fn();
vi.mock('../actions/TicketActions', () => ({
  TicketActions: (props: any) => {
    mountedActions(props);
    return (
      <div
        data-testid={`ticket-actions-${props.ticket.id}`}
        data-status={props.ticket.status}
        data-teller={props.ticket.tellerId}
      />
    );
  },
}));

const ME = { userId: 'u-me', permissions: ['QueueOperate'] };

function makeStore(initial: {
  state?: QueueBoardData;
  error?: QueueError;
}): QueueStore {
  const [state] = createSignal<QueueBoardData | undefined>(initial.state);
  const [error] = createSignal<QueueError | undefined>(initial.error);
  const refresh = vi.fn(async () => {});
  return {
    state,
    error,
    lastUpdated: () => undefined,
    version: () => undefined,
    now: () => Date.now(),
    refresh,
  };
}

function fakeCommands(): any {
  return {
    issueWalkIn: vi.fn(),
    callNext: vi.fn(),
    ticket: vi.fn(),
    cancel: vi.fn(),
    counter: vi.fn(),
    queue: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
  mountedActions.mockClear();
});

function mount(el: () => JSX.Element) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}

describe('Board component (/desk/queue board)', () => {
  it('row 1: state() undefined (first read pending) shows loading text', () => {
    const store = makeStore({ state: undefined });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);
    expect(host.textContent).toContain(th.loading);
  });

  it('row 2: store.error() = { kind: "offline" } shows an offline notice', () => {
    const store = makeStore({ error: { kind: 'offline' } });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);
    expect(host.textContent).toContain(th.offline);
    expect(host.querySelector('.desk-error')?.textContent).toContain(th.offline);
  });

  it('row 3: waiting = [B001 BOOKING, A005 WALK_IN] shows two rows in order with displayNumber and BOOKING/WALK_IN label', () => {
    const store = makeStore({
      state: {
        waiting: [
          { id: 'w-1', displayNumber: 'B001', serviceType: 'BOOKING', sequence: 1, issuedAt: '2026-10-19T08:00:00Z' },
          { id: 'w-2', displayNumber: 'A005', serviceType: 'WALK_IN', sequence: 2, issuedAt: '2026-10-19T08:01:00Z' },
        ],
        called: [],
        myCounter: null,
      },
    });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);

    const rows = host.querySelectorAll('li[data-ticket]');
    expect(rows.length).toBe(2);
    expect(rows[0]?.getAttribute('data-ticket')).toBe('w-1');
    expect(rows[0]?.textContent).toContain('B001');
    expect(rows[0]?.textContent).toContain('BOOKING');

    expect(rows[1]?.getAttribute('data-ticket')).toBe('w-2');
    expect(rows[1]?.textContent).toContain('A005');
    expect(rows[1]?.textContent).toContain('WALK_IN');
  });

  it('row 4: waiting empty shows an empty text', () => {
    const store = makeStore({
      state: {
        waiting: [],
        called: [],
        myCounter: null,
      },
    });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);
    expect(host.textContent).toContain(th.emptyWaiting);
  });

  it('row 5: called = [A003 CALLED by u-me, A004 SERVING by u-x] shows two rows with number and status, and teller row marked mine', () => {
    const store = makeStore({
      state: {
        waiting: [],
        called: [
          { id: 'c-1', displayNumber: 'A003', serviceType: 'WALK_IN', status: 'CALLED', tellerId: 'u-me', calledAt: '2026-10-19T08:02:00Z' },
          { id: 'c-2', displayNumber: 'A004', serviceType: 'WALK_IN', status: 'SERVING', tellerId: 'u-x', calledAt: '2026-10-19T08:03:00Z' },
        ],
        myCounter: null,
      },
    });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);

    const calledRows = host.querySelectorAll('li[data-ticket]');
    expect(calledRows.length).toBe(2);

    const row1 = host.querySelector("li[data-ticket='c-1']");
    expect(row1?.textContent).toContain('A003');
    expect(row1?.textContent).toContain('CALLED');
    expect(row1?.textContent).toContain(th.mine);

    const row2 = host.querySelector("li[data-ticket='c-2']");
    expect(row2?.textContent).toContain('A004');
    expect(row2?.textContent).toContain('SERVING');
    expect(row2?.textContent).not.toContain(th.mine);
  });

  it('row 6: any row mounts TicketActions with that ticket and onDone refreshes store', () => {
    const store = makeStore({
      state: {
        waiting: [
          { id: 'w-1', displayNumber: 'B001', serviceType: 'BOOKING', sequence: 1, issuedAt: '2026-10-19T08:00:00Z' },
        ],
        called: [
          { id: 'c-1', displayNumber: 'A003', serviceType: 'WALK_IN', status: 'CALLED', tellerId: 'u-me', calledAt: '2026-10-19T08:02:00Z' },
        ],
        myCounter: null,
      },
    });
    const commands = fakeCommands();
    mount(() => <Board store={store} me={ME} commands={commands} />);

    expect(mountedActions).toHaveBeenCalledTimes(2);

    const waitingCall = mountedActions.mock.calls.find((call: any[]) => call[0].ticket.id === 'w-1')!;
    expect(waitingCall).toBeDefined();
    expect(waitingCall[0].ticket).toEqual({
      id: 'w-1',
      displayNumber: 'B001',
      status: 'WAITING',
      tellerId: null,
    });
    expect(waitingCall[0].me).toBe(ME);
    expect(waitingCall[0].commands).toBe(commands);

    const calledCall = mountedActions.mock.calls.find((call: any[]) => call[0].ticket.id === 'c-1')!;
    expect(calledCall).toBeDefined();
    expect(calledCall[0].ticket).toEqual({
      id: 'c-1',
      displayNumber: 'A003',
      status: 'CALLED',
      tellerId: 'u-me',
    });
    expect(calledCall[0].me).toBe(ME);
    expect(calledCall[0].commands).toBe(commands);

    waitingCall[0].onDone();
    expect(store.refresh).toHaveBeenCalledTimes(1);
  });
});
