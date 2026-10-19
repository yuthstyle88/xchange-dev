// B06 per-ticket actions (FE-desk skeleton stub, #564): call, recall, skip, start, finish and cancel with a reason, as
// the ticket's status and the caller allow. Filled by its own ticket.
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../../shared/api/client';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import type { QueueCommands, TicketVerb } from '../queue-commands';
import type { QueueMe } from '../QueueScreen';
import { en, th } from './i18n';

const t = screenT('desk.queue.actions', { th, en });

const OWN_ERROR_KEYS = {
  notYourTicket: 'error.notYourTicket',
  counterPaused: 'error.counterPaused',
  noOpenShift: 'error.noOpenShift',
  staleVersion: 'error.staleVersion',
} as const satisfies Record<string, keyof typeof th>;

function refusalMessage(err: unknown): string {
  if (err instanceof ApiError && err.code in OWN_ERROR_KEYS) {
    return t(OWN_ERROR_KEYS[err.code as keyof typeof OWN_ERROR_KEYS]);
  }
  return wordedError(err);
}

/** The board row a set of actions acts on: a WAITING, CALLED or SERVING ticket. */
export interface ActionTicket {
  id: string;
  displayNumber: string;
  /** `WAITING`, `CALLED` or `SERVING`. */
  status: string;
  /** The teller whose counter holds it; `null` while WAITING. */
  tellerId: string | null;
}

export interface TicketActionsProps {
  ticket: ActionTicket;
  me: QueueMe;
  commands: QueueCommands;
  /** Called after any command answered, so the board reads again. */
  onDone(): void;
}

export function TicketActions(props: TicketActionsProps): JSX.Element {
  const [busy, setBusy] = createSignal(false);
  const [cancelling, setCancelling] = createSignal(false);
  const [reason, setReason] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);

  const mineOrManager = () =>
    props.ticket.tellerId === props.me.userId ||
    props.me.permissions.includes('BranchDayOperate');

  const verbs = (): TicketVerb[] => {
    const status = props.ticket.status;
    if (status === 'WAITING') return ['call'];
    if (status === 'CALLED') {
      return mineOrManager() ? ['recall', 'skip', 'start'] : ['skip'];
    }
    if (status === 'SERVING') {
      return mineOrManager() ? ['finish'] : [];
    }
    return [];
  };

  const canCancel = () =>
    props.ticket.status === 'WAITING' || props.ticket.status === 'CALLED';

  async function run(work: () => Promise<unknown>): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await work();
      setCancelling(false);
      setReason('');
      props.onDone();
    } catch (err) {
      setError(refusalMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function submitCancel(): void {
    const trimmed = reason().trim();
    if (!trimmed) {
      setError(t('error.cancelReasonRequired'));
      return;
    }
    void run(() => props.commands.cancel(props.ticket.id, trimmed));
  }

  return (
    <Show when={verbs().length > 0 || canCancel()}>
      <div class="desk-actions">
        <For each={verbs()}>
          {(verb) => (
            <button
              type="button"
              name={verb}
              disabled={busy()}
              onClick={() => void run(() => props.commands.ticket(verb, props.ticket.id))}
            >
              {t(verb)}
            </button>
          )}
        </For>
        <Show when={canCancel()}>
          <button
            type="button"
            name="cancel"
            disabled={busy()}
            onClick={() => {
              setError(null);
              setCancelling(true);
            }}
          >
            {t('cancel')}
          </button>
        </Show>
        <Show when={cancelling()}>
          <label class="desk-field">
            <span>{t('cancelReason')}</span>
            <input
              type="text"
              name="cancelReason"
              value={reason()}
              disabled={busy()}
              onInput={(e) => setReason(e.currentTarget.value)}
            />
          </label>
          <button
            type="button"
            name="cancelConfirm"
            disabled={busy()}
            onClick={submitCancel}
          >
            {t('cancelConfirm')}
          </button>
        </Show>
        <Show when={error()}>
          {(msg) => (
            <p class="desk-error" role="alert">
              {msg()}
            </p>
          )}
        </Show>
      </div>
    </Show>
  );
}
