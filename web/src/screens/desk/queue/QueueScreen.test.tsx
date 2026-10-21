// B06 `/desk/queue` screen test (#4040): staff_me loading, permissions passed to CounterBar, and missing data defaults.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { QueueStore } from '../../../shared/queue/queue-store';
import {
  makeQueueBoardCalled,
  makeQueueBoardData,
  type QueueBoardData,
} from '../../../shared/testing/fixtures';
import { th as boardTh } from './board/i18n';
import { th } from './i18n';
import { QueueScreen } from './QueueScreen';

const fake = vi.hoisted(() => ({
  view: vi.fn(),
  command: vi.fn(),
  setState: undefined as undefined | ((v: QueueBoardData | undefined) => void),
}));

vi.mock('../../../shared/session/session', () => ({
  sharedApi: () => ({ view: fake.view, command: fake.command }),
}));

vi.mock('../../../shared/queue/queue-store', async () => {
  const { createSignal } = await import('solid-js');
  const [state, setState] = createSignal<QueueBoardData | undefined>(undefined);
  fake.setState = (v) => setState(() => v);
  const store: QueueStore = {
    state,
    error: () => undefined,
    lastUpdated: () => undefined,
    version: () => undefined,
    now: () => Date.now(),
    refresh: async () => {},
  };
  return {
    queueBranch: () => 'B1',
    useQueueStore: () => () => store,
  };
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  fake.view.mockReset();
  fake.command.mockReset();
  fake.setState?.(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

function mount(el: () => JSX.Element): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe('QueueScreen (/desk/queue)', () => {
  it('row 1: staff_me pending shows the loadingMe text (กำลังโหลด...)', () => {
    fake.view.mockReturnValue(new Promise(() => {}));
    mount(() => <QueueScreen />);
    expect(fake.view).toHaveBeenCalledWith('staff_me', {});
    expect(host.textContent).toContain(th.loadingMe);
    expect(host.querySelector('button[name=issueWalkIn]')).toBeNull();
  });

  it('row 2: {userId: "u1", permissions: ["BranchDayOperate"]} shows button[name=openQueue]', async () => {
    fake.view.mockResolvedValue({
      version: 1,
      changed: true,
      data: { userId: 'u1', permissions: ['BranchDayOperate'] },
    });
    mount(() => <QueueScreen />);
    await flush();
    expect(host.querySelector('button[name=openQueue]')).not.toBeNull();
  });

  it('row 3: {userId: "u1", permissions: []} hides openQueue button', async () => {
    fake.view.mockResolvedValue({
      version: 1,
      changed: true,
      data: { userId: 'u1', permissions: [] },
    });
    mount(() => <QueueScreen />);
    await flush();
    expect(host.querySelector('button[name=issueWalkIn]')).not.toBeNull();
    expect(host.querySelector('button[name=openQueue]')).toBeNull();
  });

  it('row 4: staff_me data missing renders with userId "" and permissions [] without throwing', async () => {
    fake.setState?.(
      makeQueueBoardData({
        waiting: [],
        called: [
          makeQueueBoardCalled({
            id: 'c-1',
            displayNumber: 'A001',
            serviceType: 'WALK_IN',
            status: 'CALLED',
            tellerId: '',
            calledAt: '2026-10-21T00:00:00Z',
          }),
        ],
        myCounter: null,
      }),
    );
    fake.view.mockResolvedValue({ version: 1, changed: false });
    mount(() => <QueueScreen />);
    await flush();
    expect(host.querySelector('button[name=issueWalkIn]')).not.toBeNull();
    expect(host.querySelector('button[name=openQueue]')).toBeNull();
    expect(host.querySelector("li[data-ticket='c-1']")?.textContent).toContain(boardTh.mine);
  });
});
