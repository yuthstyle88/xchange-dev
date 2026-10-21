// B06 `/desk/queue` (design docs/design/blueprint/queue-tickets.md §4.4, §5; FE-desk skeleton, #564). The screen only
// composes its parts: the board is read through the shared queue store (`queue_board`, polled with `since`;
// shared/queue/queue-store.ts:274), the caller is read once from `staff_me`, and every write goes through
// `queueCommands`. Each part lives in its own folder and is its own ticket.
import { createResource, Show, type JSX } from 'solid-js';
import { sharedApi } from '../../../shared/session/session';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { queueBranch, useQueueStore } from '../../../shared/queue/queue-store';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Board } from './board/Board';
import { CounterBar } from './counter/CounterBar';
import { queueCommands } from './queue-commands';
import { en, th } from './i18n';

const t = screenT('desk.queue', { th, en });

/** What the parts need to know about the caller. */
export interface QueueMe {
  /** `users.id`, compared with a ticket's `tellerId`. */
  userId: string;
  permissions: readonly string[];
}

export function QueueScreen(): JSX.Element {
  const api = sharedApi();
  const store = useQueueStore(queueBranch);
  const commands = queueCommands(api);
  const [me, { refetch }] = createResource(async (): Promise<QueueMe> => {
    const answer = await api.view<StaffMeView>('staff_me', {});
    return { userId: answer.data?.userId ?? '', permissions: answer.data?.permissions ?? [] };
  });

  return (
    <Show
      when={!me.error}
      fallback={
        <p role="alert">
          {wordedError(me.error)}{' '}
          <button type="button" onClick={() => refetch()}>
            {t('retry')}
          </button>
        </p>
      }
    >
      <Show when={me()} fallback={<p>{t('loadingMe')}</p>}>
        {(m) => (
          <div>
            <CounterBar store={store()} me={m()} commands={commands} />
            <Board store={store()} me={m()} commands={commands} />
          </div>
        )}
      </Show>
    </Show>
  );
}
