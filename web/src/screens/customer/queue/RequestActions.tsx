// QW-2f (kiosk-shared-phone §5r F8, R-11; rows U4, U5 of §5r.3; §5r.4a): the request's buttons. Skeleton by
// QW-2.0; the body is QW-2f's.
//
// - button[data-action="cancel-web-request"] (ยกเลิกคิว): one press calls `onCancel` once, with no dialog and no
//   confirm(); it is not rendered once `status` is CANCELLED.
// - button[data-action="done"] (เสร็จ): calls `onDone` once.
// - button[data-action="email-copy"] (ส่งเข้าอีเมล) only when `emailCopy.available`, with the masked `emailCopy.to`
//   shown beside it. A press calls `onEmailCopy` once; then a `role="status"` line says it was sent. A refusal
//   shows a worded `role="alert"` line (wordedError).
// - A button whose call is in flight is disabled; a second press while in flight calls nothing.
// Classes: xc-queue-actions, xc-queue-cancel, xc-queue-done, xc-queue-email, xc-queue-email-to, xc-queue-email-sent,
// xc-queue-error. Texts: cancel, done, emailCopy, emailTo, emailSent.
import { createSignal, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import type { EmailCopy, EmailCopyAnswer, WebRequestStatus } from './queue-api';

const t = screenT('customer.queue', table);

export interface RequestActionsProps {
  emailCopy: EmailCopy;
  /** The shown status (the state's, else null): no cancel button once CANCELLED. */
  status: WebRequestStatus | null;
  onCancel: () => Promise<void>;
  onDone: () => Promise<void>;
  onEmailCopy: () => Promise<EmailCopyAnswer>;
}

export function RequestActions(props: RequestActionsProps): JSX.Element {
  const [cancelling, setCancelling] = createSignal(false);
  const [doneBusy, setDoneBusy] = createSignal(false);
  const [emailBusy, setEmailBusy] = createSignal(false);
  const [emailSent, setEmailSent] = createSignal(false);
  const [emailError, setEmailError] = createSignal<string | null>(null);

  const handleCancel = async () => {
    if (cancelling()) return;
    setCancelling(true);
    try {
      await props.onCancel();
    } finally {
      setCancelling(false);
    }
  };

  const handleDone = async () => {
    if (doneBusy()) return;
    setDoneBusy(true);
    try {
      await props.onDone();
    } finally {
      setDoneBusy(false);
    }
  };

  const handleEmailCopy = async () => {
    if (emailBusy()) return;
    setEmailBusy(true);
    setEmailSent(false);
    setEmailError(null);
    try {
      const ans = await props.onEmailCopy();
      if (ans.sent) {
        setEmailSent(true);
      }
    } catch (err) {
      setEmailError(wordedError(err));
    } finally {
      setEmailBusy(false);
    }
  };

  return (
    <div class="xc-queue-actions">
      <Show when={props.status !== 'CANCELLED'}>
        <button
          type="button"
          class="xc-queue-cancel"
          data-action="cancel-web-request"
          disabled={cancelling()}
          onClick={() => void handleCancel()}
        >
          {t('cancel')}
        </button>
      </Show>
      <button
        type="button"
        class="xc-queue-done"
        data-action="done"
        disabled={doneBusy()}
        onClick={() => void handleDone()}
      >
        {t('done')}
      </button>
      <Show when={props.emailCopy.available}>
        <button
          type="button"
          class="xc-queue-email"
          data-action="email-copy"
          disabled={emailBusy()}
          onClick={() => void handleEmailCopy()}
        >
          {t('emailCopy')}
        </button>
        <span class="xc-queue-email-to">{t('emailTo', { to: props.emailCopy.to ?? '' })}</span>
      </Show>
      <Show when={emailSent()}>
        <p class="xc-queue-email-sent" role="status">
          {t('emailSent')}
        </p>
      </Show>
      <Show when={emailError()}>
        {(msg) => (
          <p class="xc-queue-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
    </div>
  );
}
