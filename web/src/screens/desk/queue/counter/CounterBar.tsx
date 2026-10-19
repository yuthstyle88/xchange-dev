// B06 counter bar (FE-desk skeleton stub, #564): issue a walk-in ticket, call next, pause or resume my counter, and open
// or close the branch queue (BranchDayOperate only). Filled by its own ticket.
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../../shared/api/client';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import type { QueueStore } from '../../../../shared/queue/queue-store';
import type { QueueCommands } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';
import { en, th } from './i18n';

const t = screenT('desk.queue.counter', { th, en });

const OWN_ERROR_KEYS = {
  nothingWaiting: 'error.nothingWaiting',
  queueClosed: 'error.queueClosed',
  noOpenShift: 'error.noOpenShift',
  counterPaused: 'error.counterPaused',
} as const satisfies Record<string, keyof typeof th>;

function refusalMessage(err: unknown): string {
  if (err instanceof ApiError && err.code in OWN_ERROR_KEYS) {
    return t(OWN_ERROR_KEYS[err.code as keyof typeof OWN_ERROR_KEYS]);
  }
  return wordedError(err);
}

export interface CounterBarProps {
  store: QueueStore;
  me: QueueMe;
  commands: QueueCommands;
}

export function CounterBar(props: CounterBarProps): JSX.Element {
  const [busy, setBusy] = createSignal(false);
  const [lastNumber, setLastNumber] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const myCounter = () => props.store.state()?.myCounter ?? null;
  const isActive = () => myCounter()?.status === 'ACTIVE';
  const isPaused = () => myCounter()?.status === 'PAUSED';
  const isManager = () => props.me.permissions.includes('BranchDayOperate');

  async function run(work: () => Promise<void>): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (err) {
      setError(refusalMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function runTicket(send: () => Promise<{ displayNumber: string }>): void {
    void run(async () => {
      const ticket = await send();
      setLastNumber(ticket.displayNumber);
      await props.store.refresh();
    });
  }

  return (
    <div class="desk-actions">
      <button
        type="button"
        name="issueWalkIn"
        disabled={busy()}
        onClick={() => runTicket(() => props.commands.issueWalkIn())}
      >
        {t('issueWalkIn')}
      </button>
      <button
        type="button"
        name="callNext"
        disabled={!isActive() || busy()}
        onClick={() => runTicket(() => props.commands.callNext())}
      >
        {t('callNext')}
      </button>
      <Show when={isActive()}>
        <button
          type="button"
          name="pause"
          disabled={busy()}
          onClick={() =>
            void run(async () => {
              await props.commands.counter('pause');
              await props.store.refresh();
            })
          }
        >
          {t('pause')}
        </button>
      </Show>
      <Show when={isPaused()}>
        <button
          type="button"
          name="resume"
          disabled={busy()}
          onClick={() =>
            void run(async () => {
              await props.commands.counter('resume');
              await props.store.refresh();
            })
          }
        >
          {t('resume')}
        </button>
      </Show>
      <Show when={isManager()}>
        <button
          type="button"
          name="openQueue"
          disabled={busy()}
          onClick={() =>
            void run(async () => {
              await props.commands.queue('open');
              await props.store.refresh();
            })
          }
        >
          {t('openQueue')}
        </button>
        <button
          type="button"
          name="closeQueue"
          disabled={busy()}
          onClick={() =>
            void run(async () => {
              await props.commands.queue('close');
              await props.store.refresh();
            })
          }
        >
          {t('closeQueue')}
        </button>
      </Show>
      <Show when={myCounter() === null}>
        <p class="desk-notice">{t('noCounter')}</p>
      </Show>
      <Show when={lastNumber()}>
        {(num) => (
          <p class="desk-notice" role="status">
            {num()}
          </p>
        )}
      </Show>
      <Show when={error()}>
        {(msg) => (
          <p class="desk-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
    </div>
  );
}
