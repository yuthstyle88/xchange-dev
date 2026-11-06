// F28a (#6338): desk queue board shows the kiosk K-number beside its ticket when kioskCode is present.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { Board } from './Board';
import {
  makeQueueBoardCalled,
  makeQueueBoardData,
  makeQueueBoardWaiting,
  type QueueBoardData,
} from '../../../../shared/testing/fixtures';
import type { QueueError, QueueStore } from '../../../../shared/queue/queue-store';

vi.mock('../actions/TicketActions', () => ({
  TicketActions: () => <div data-testid="ticket-actions" />,
}));

const ME = { userId: 'u-me', permissions: ['QueueOperate'] };

function makeStore(initial: {
  state?: QueueBoardData;
  error?: QueueError;
}): QueueStore {
  const [state] = createSignal<QueueBoardData | undefined>(initial.state);
  const [error] = createSignal<QueueError | undefined>(initial.error);
  return {
    state,
    error,
    lastUpdated: () => undefined,
    version: () => undefined,
    now: () => Date.now(),
    refresh: vi.fn(async () => {}),
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
});

function mount(el: () => JSX.Element) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}

describe('Board kiosk code display (#6338)', () => {
  it('shows the K-code beside a waiting kiosk ticket number, and nothing extra beside a walk-in', () => {
    const store = makeStore({
      state: makeQueueBoardData({
        waiting: [
          makeQueueBoardWaiting({
            id: 'w-kiosk',
            displayNumber: 'W001',
            serviceType: 'WALK_IN',
            kioskCode: 'K-6CEE4945',
          }),
          makeQueueBoardWaiting({
            id: 'w-walkin',
            displayNumber: 'W002',
            serviceType: 'WALK_IN',
            kioskCode: null,
          }),
        ],
        called: [],
        myCounter: null,
      }),
    });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);

    const kioskRow = host.querySelector("li[data-ticket='w-kiosk']")!;
    expect(kioskRow).toBeDefined();
    expect(kioskRow.textContent).toContain('W001 · K-6CEE4945');
    const kioskSpan = kioskRow.querySelector('[data-kiosk-code]');
    expect(kioskSpan).not.toBeNull();
    expect(kioskSpan?.textContent).toContain('K-6CEE4945');

    const walkInRow = host.querySelector("li[data-ticket='w-walkin']")!;
    expect(walkInRow).toBeDefined();
    expect(walkInRow.textContent).toContain('W002');
    expect(walkInRow.textContent).not.toContain('·');
    expect(walkInRow.querySelector('[data-kiosk-code]')).toBeNull();
  });

  it('shows the K-code beside a called kiosk ticket number, and nothing extra beside a walk-in', () => {
    const store = makeStore({
      state: makeQueueBoardData({
        waiting: [],
        called: [
          makeQueueBoardCalled({
            id: 'c-kiosk',
            displayNumber: 'W001',
            serviceType: 'WALK_IN',
            status: 'CALLED',
            kioskCode: 'K-6CEE4945',
          }),
          makeQueueBoardCalled({
            id: 'c-walkin',
            displayNumber: 'W002',
            serviceType: 'WALK_IN',
            status: 'CALLED',
            kioskCode: null,
          }),
        ],
        myCounter: null,
      }),
    });
    mount(() => <Board store={store} me={ME} commands={fakeCommands()} />);

    const kioskRow = host.querySelector("li[data-ticket='c-kiosk']")!;
    expect(kioskRow).toBeDefined();
    expect(kioskRow.textContent).toContain('W001 · K-6CEE4945');
    const kioskSpan = kioskRow.querySelector('[data-kiosk-code]');
    expect(kioskSpan).not.toBeNull();
    expect(kioskSpan?.textContent).toContain('K-6CEE4945');

    const walkInRow = host.querySelector("li[data-ticket='c-walkin']")!;
    expect(walkInRow).toBeDefined();
    expect(walkInRow.textContent).toContain('W002');
    expect(walkInRow.textContent).not.toContain('·');
    expect(walkInRow.querySelector('[data-kiosk-code]')).toBeNull();
  });
});
