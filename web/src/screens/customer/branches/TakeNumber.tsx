// C08: take a queue number from the app, `queueTicketTake` `POST /api/app/queue/tickets` `{branch}`
// (Blueprint C08 "จองหรือรับคิว"; section-b-gaps.md S7; BE contract #3410).
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError, type Fetch } from '../../../shared/api/client';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { en, th, zh } from './i18n';

export interface TakeNumberProps {
  /** The branch id (`PublicBranchDto.id`); views and commands match it against `branches.id`. */
  branch: string;
  /** `PublicBranchDto.queueOpen`: no button when the queue is closed. */
  queueOpen: boolean;
  /** The screen's fetch, so tests mock at the fetch level. */
  fetch: Fetch;
}

interface TakeErrorState {
  message: string;
  linkHref?: string;
  linkText?: string;
}

export function TakeNumber(props: TakeNumberProps): JSX.Element {
  const t = screenT('customer.branches', { th, en, zh });
  const writer = keyedWrite<{ branch: string }, MyQueueTicketData>(
    new ApiClient(props.fetch),
    'POST',
    '/api/app/queue/tickets',
  );

  const [inFlight, setInFlight] = createSignal(false);
  const [ticket, setTicket] = createSignal<MyQueueTicketData | undefined>(undefined);
  const [error, setError] = createSignal<TakeErrorState | undefined>(undefined);

  const handleTake = async () => {
    if (inFlight() || ticket()) return;
    setInFlight(true);
    setError(undefined);
    try {
      const created = await writer.send({ branch: props.branch });
      setTicket(created);
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        if (err.status === 401) {
          setError({
            message: t('signInToTake'),
            linkHref: '/app/sign-in',
            linkText: t('signInToTake'),
          });
        } else if (err.code === 'alreadyQueued') {
          setError({
            message: t('alreadyQueued'),
            linkHref: `/app/journey?branch=${encodeURIComponent(props.branch)}`,
            linkText: t('viewTicket'),
          });
        } else if (err.code === 'queueClosed') {
          setError({ message: t('queueClosedNow') });
        } else {
          setError({ message: t('takeFailed') });
        }
      } else {
        setError({ message: t('takeFailed') });
      }
    } finally {
      setInFlight(false);
    }
  };

  return (
    <div class="xc-take-number">
      <Show when={props.queueOpen && !ticket()}>
        <button
          type="button"
          class="xc-take-number-btn"
          data-action="take-number"
          disabled={inFlight()}
          onClick={() => void handleTake()}
        >
          {t('takeNumber')}
        </button>
      </Show>

      <Show when={ticket()}>
        {(tk) => (
          <>
            <span class="xc-take-number-message" data-testid="take-number-message">
              {t('takenNumber', { number: tk().displayNumber })}
            </span>
            <a
              class="xc-take-number-link"
              data-testid="take-number-link"
              href={`/app/journey?branch=${encodeURIComponent(props.branch)}&ticketId=${encodeURIComponent(tk().ticketId)}`}
            >
              {t('viewTicket')}
            </a>
          </>
        )}
      </Show>

      <Show when={!ticket() && error()}>
        {(err) => (
          <>
            <span class="xc-take-number-message" data-testid="take-number-message">
              {err().message}
            </span>
            <Show when={err().linkHref}>
              {(href) => (
                <a class="xc-take-number-link" data-testid="take-number-link" href={href()}>
                  {err().linkText}
                </a>
              )}
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
