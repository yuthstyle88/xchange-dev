// B06 board (FE-desk skeleton stub, #564): the WAITING tickets in call order and the CALLED/SERVING ones with their
// teller, from `queue_board`. Each row mounts `TicketActions`. Filled by its own ticket.
import { For, Show, type JSX } from 'solid-js';
import type { QueueStore } from '../../../../shared/queue/queue-store';
import type { QueueCommands } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';
import { screenT } from '../../../../shared/i18n';
import { TicketActions } from '../actions/TicketActions';
import { en, th } from './i18n';

export interface BoardProps {
  store: QueueStore;
  me: QueueMe;
  commands: QueueCommands;
}

export function Board(props: BoardProps): JSX.Element {
  const t = screenT('desk.queue.board', { th, en });
  const error = () => props.store.error();
  const state = () => props.store.state();

  return (
    <div class="desk-queue-board">
      <Show when={error()?.kind === 'offline'}>
        <p role="status" class="desk-error">{t('offline')}</p>
      </Show>
      <Show
        when={state()}
        fallback={
          <Show when={error()?.kind !== 'offline'}>
            <p>{t('loading')}</p>
          </Show>
        }
      >
        {(s) => (
          <div class="desk-queue-sections">
            <section class="card desk-queue-section">
              <h3>{t('waiting')}</h3>
              <Show when={s().waiting.length > 0} fallback={<p class="desk-empty">{t('emptyWaiting')}</p>}>
                <ul class="desk-queue-list">
                  <For each={s().waiting}>
                    {(w) => (
                      <li class="desk-queue-row" data-ticket={w.id}>
                        <span>{w.displayNumber}</span>
                        <Show when={w.kioskCode}>
                          {(code) => (
                            <span data-kiosk-code={code()}>
                              {' · '}
                              {code()}
                            </span>
                          )}
                        </Show>{' '}
                        <span>{w.serviceType === 'BOOKING' ? t('booking') : t('walkIn')}</span>
                        <TicketActions
                          ticket={{
                            id: w.id,
                            displayNumber: w.displayNumber,
                            status: 'WAITING',
                            tellerId: null,
                          }}
                          me={props.me}
                          commands={props.commands}
                          onDone={() => void props.store.refresh()}
                        />
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </section>
            <section class="card desk-queue-section">
              <h3>{t('nowServing')}</h3>
              <Show when={s().called.length > 0} fallback={<p class="desk-empty">{t('emptyCalled')}</p>}>
                <ul class="desk-queue-list">
                  <For each={s().called}>
                    {(c) => (
                      <li class="desk-queue-row" data-ticket={c.id}>
                        <span>{c.displayNumber}</span>
                        <Show when={c.kioskCode}>
                          {(code) => (
                            <span data-kiosk-code={code()}>
                              {' · '}
                              {code()}
                            </span>
                          )}
                        </Show>{' '}
                        <span>{c.status === 'CALLED' ? t('called') : c.status === 'SERVING' ? t('serving') : c.status}</span>{' '}
                        <Show when={c.tellerId === props.me.userId}>
                          <span>{t('mine')}</span>
                        </Show>
                        <TicketActions
                          ticket={{
                            id: c.id,
                            displayNumber: c.displayNumber,
                            status: c.status,
                            tellerId: c.tellerId,
                          }}
                          me={props.me}
                          commands={props.commands}
                          onDone={() => void props.store.refresh()}
                        />
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </section>
          </div>
        )}
      </Show>
    </div>
  );
}
