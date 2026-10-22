// G-ME (`docs/design/web-silent-failures.md` §5): the one worded line a staff or admin screen shows when its `me` read
// (the permissions) failed, instead of hiding its permission-gated actions. The #1719 standard (`CashFlows.tsx`): the
// failure is worded, and every gated action stays on screen, disabled, with `permissionsUnavailableText(error)` as its
// reason (`title`).
//
// STUB by the skeleton (claude/e8020026's sub-agent). GME-0 fills both bodies. Keep the names, the props and the hooks:
//   - `PermissionsUnavailable({ error, onRetry, busy })`:
//       error null or undefined ⇒ renders nothing at all;
//       otherwise ⇒ `<p role="alert" class="xc-perm-unavailable" data-permissions-unavailable="">` holding
//       `permissionsUnavailableText(error)`, and inside it one
//       `<button type="button" class="xc-perm-unavailable-retry" data-action="permissions-retry">` reading
//       `t('shell.permissions.retry')`, disabled while `busy`, whose click calls `onRetry()` once.
//   - `permissionsUnavailableText(error)`: `t('shell.permissions.unavailable')`, one space, then `wordedError(error)`
//       (so a network failure and a server failure read differently). Reactive to `setLang` when read in JSX.
// The retry re-reads `me` through whoever owns the read: `sharedSession().retryMe` for a screen on the session store,
// or the screen's own `me` read where its design pins one (TransactionScreen D2, ReferralCodeField's `api` prop).
import { Show, type JSX } from 'solid-js';
import { t } from '../i18n';
import { wordedError } from '../i18n/worded-error';
import './permissions-unavailable.css';

export interface PermissionsUnavailableProps {
  /** The failure of the `me` read; null or undefined renders nothing. */
  error: unknown;
  /** Re-reads `me` (see the header). */
  onRetry: () => void;
  /** True while that re-read is in flight: the retry button is disabled. */
  busy?: boolean;
}

/** The worded reason, for the line and for each disabled action's `title`. */
export function permissionsUnavailableText(error: unknown): string {
  return `${t('shell.permissions.unavailable')} ${wordedError(error)}`;
}

export function PermissionsUnavailable(props: PermissionsUnavailableProps): JSX.Element {
  return (
    <Show when={props.error !== null && props.error !== undefined}>
      <p role="alert" class="xc-perm-unavailable" data-permissions-unavailable="">
        <span>{permissionsUnavailableText(props.error)}</span>
        <button
          type="button"
          class="xc-perm-unavailable-retry"
          data-action="permissions-retry"
          disabled={Boolean(props.busy)}
          onClick={() => props.onRetry()}
        >
          {t('shell.permissions.retry')}
        </button>
      </p>
    </Show>
  );
}
