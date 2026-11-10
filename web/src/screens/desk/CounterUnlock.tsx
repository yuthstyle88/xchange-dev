// XC Counter kiosk exit (docs/design/xc-counter-app.md §8.8, xchange#6569).
// - Inside the app (`window.xcCounter`): a link to the app's own exit page, where staff type a manager's code.
// - For a holder of `counterUnlock` (manager, owner): pick a counter of the session's branch and issue its one-time
//   code (`counterUnlockIssue`), shown once with the time it stops working.
// A refusal is `wordedError` in a `role="alert"`.
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import type {
  CounterUnlockIssueInput,
  CounterUnlockIssueOutput,
} from '../../shared/api/generated/counter_unlock_issue';
import { keyedWrite } from '../../shared/api/keyed-write';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { getXcCounter } from '../../shared/xc-counter/bridge';
import {
  BRANCH_COUNTERS_VIEW,
  type BranchCounterRow,
  type BranchCountersData,
} from '../admin/branches/counters-api';
import * as screenI18n from './i18n';

export const COUNTER_UNLOCK_ISSUE = '/api/v3/commands/counter_unlock_issue';
/** The app's local exit page (`counter-app/ui/exit.html`); `navigation.rs` lets it through. */
export const COUNTER_EXIT_PAGE = 'tauri://localhost/exit.html';

export function CounterUnlock(props: {
  branchId?: string;
  canIssue: boolean;
  api?: ApiClient;
}): JSX.Element {
  const t = screenT('desk', screenI18n);
  const api = props.api ?? new ApiClient();
  const issue = keyedWrite<CounterUnlockIssueInput, CounterUnlockIssueOutput>(
    api,
    'POST',
    COUNTER_UNLOCK_ISSUE,
  );
  const [counters, setCounters] = createSignal<BranchCounterRow[]>([]);
  const [agentId, setAgentId] = createSignal('');
  const [issued, setIssued] = createSignal<CounterUnlockIssueOutput | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const load = async () => {
    if (!props.branchId || counters().length) return;
    try {
      const res = await api.view<BranchCountersData>(BRANCH_COUNTERS_VIEW, {
        branch: props.branchId,
      });
      const rows = res.data?.counters ?? [];
      setCounters(rows);
      if (!agentId() && rows[0]) setAgentId(rows[0].agentId);
    } catch (err: unknown) {
      setError(wordedError(err));
    }
  };

  const send = async () => {
    setError(null);
    setIssued(null);
    try {
      setIssued(await issue.send({ agentId: agentId() }));
    } catch (err: unknown) {
      setError(wordedError(err));
    }
  };

  const until = (iso: string) => formatBangkokDateTime(iso);

  return (
    <>
      <Show when={getXcCounter()}>
        <a href={COUNTER_EXIT_PAGE} data-counter-exit-link="">
          {t('counterExitLink')}
        </a>
      </Show>
      <Show when={props.canIssue}>
        <details data-counter-unlock="" onToggle={(e) => e.currentTarget.open && void load()}>
          <summary>{t('counterUnlockHeading')}</summary>
          <label>
            {t('counterUnlockCounter')}
            <select value={agentId()} onChange={(e) => setAgentId(e.currentTarget.value)}>
              <For each={counters()}>
                {(c) => <option value={c.agentId}>{c.name || c.agentId}</option>}
              </For>
            </select>
          </label>
          <button type="button" disabled={!agentId()} onClick={() => void send()}>
            {t('counterUnlockIssue')}
          </button>
          <Show when={issued()}>
            {(out) => (
              <output data-counter-unlock-code="">
                {t('counterUnlockCode', { code: out().code, time: until(out().expiresAt) })}
              </output>
            )}
          </Show>
          <Show when={error()}>
            <p role="alert">{error()}</p>
          </Show>
        </details>
      </Show>
    </>
  );
}
