// Desk QueueScreen error and retry tests (#4059): staff_me rejection shows role=alert with worded error and retry.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { QueueStore } from '../../../shared/queue/queue-store';
import type { QueueBoardData } from '../../../shared/testing/fixtures';
import { ApiError } from '../../../shared/api/client';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { en, th } from './i18n';
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
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe('QueueScreen error handling (#4059)', () => {
  it('staff_me rejects => shows role=alert with worded error, no CounterBar, no crash', async () => {
    fake.view.mockRejectedValue(new ApiError({ status: 500, code: 'serverError', message: 'internal server error' }));

    expect(() => mount(() => <QueueScreen />)).not.toThrow();
    await flush();

    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain(sharedTh['error.serverError']);
    expect(alert?.querySelector('button')).not.toBeNull();
    expect(alert?.querySelector('button')?.textContent).toContain(th.retry);

    // Never renders CounterBar or Board while in error
    expect(host.querySelector('button[name=issueWalkIn]')).toBeNull();
    expect(host.querySelector('button[name=openQueue]')).toBeNull();
  });

  it('Retry with a now-answering staff_me => alert clears and CounterBar shows', async () => {
    fake.view.mockRejectedValueOnce(new ApiError({ status: 500, code: 'serverError', message: 'internal server error' }));

    mount(() => <QueueScreen />);
    await flush();

    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    const retryBtn = alert?.querySelector('button');
    expect(retryBtn).not.toBeNull();

    // staff_me now succeeds on retry
    fake.view.mockResolvedValueOnce({
      version: 1,
      changed: true,
      data: { userId: 'u1', permissions: ['BranchDayOperate'] },
    });

    retryBtn!.click();
    await flush();

    // Alert is gone, CounterBar is rendered
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('button[name=issueWalkIn]')).not.toBeNull();
    expect(host.querySelector('button[name=openQueue]')).not.toBeNull();
  });

  it('in English, worded error and retry button are localized', async () => {
    setLang('en');
    fake.view.mockRejectedValue(new ApiError({ status: 500, code: 'serverError', message: 'server down' }));

    mount(() => <QueueScreen />);
    await flush();

    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain(sharedEn['error.serverError']);
    expect(alert?.querySelector('button')?.textContent).toContain(en.retry);
  });
});
