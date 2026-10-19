// B03 desk phone capture + SMS code (FE-desk, #564; design membership-points.md §6.3 CU-P1; commands
// api/src/presentation/http/v3/commands/customer_phone_start.rs:29-62 and customer_phone_confirm.rs:29-70, which export
// no TS type, so the types are here).
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import { keyedWrite } from '../../../../shared/api/keyed-write';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import * as texts from './i18n';

/** `POST /api/customers/{id}/phone-verifications` body (`customer_phone_start.rs:29-33`). */
export interface PhoneStartInput {
  id: string;
  branch: string;
  phone: string;
}
/** Its answer (`customer_phone_start.rs:37-40`). */
export interface PhoneStartOutput {
  challengeId: string;
  expiresAt: string;
}
/** `POST /api/customers/{id}/phone-verifications/{challengeId}/confirm` body (`customer_phone_confirm.rs:29-35`). */
export interface PhoneConfirmInput {
  id: string;
  challengeId: string;
  branch: string;
  code: string;
}
/** Its answer (`customer_phone_confirm.rs:39-42`). */
export interface PhoneConfirmOutput {
  verified: boolean;
  identityId: string;
}

export interface PhoneVerifyProps {
  api: ApiClient;
  branch: string;
  customerId: string;
  /** Called once the server answers `verified: true`. */
  onVerified(): void;
}

const t = screenT('desk.loyalty.phone', texts);

const PHONE_ERROR_CODES = new Set([
  'wrongCode',
  'codeExpired',
  'tooManyAttempts',
  'rateLimited',
  'phoneInUse',
  'smsUnavailable',
] as const);

function refusalText(err: unknown): string {
  if (err instanceof ApiError && PHONE_ERROR_CODES.has(err.code as never)) {
    return t(`error.${err.code}` as keyof typeof texts.th);
  }
  return wordedError(err);
}

export function PhoneVerify(props: PhoneVerifyProps): JSX.Element {
  const [phone, setPhone] = createSignal('');
  const [code, setCode] = createSignal('');
  const [challengeId, setChallengeId] = createSignal<string | undefined>();
  const [verified, setVerified] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  let codeInputRef: HTMLInputElement | undefined;

  const startWrite = keyedWrite<PhoneStartInput, PhoneStartOutput>(
    props.api,
    'POST',
    (b) => `/api/customers/${b.id}/phone-verifications`,
  );
  const confirmWrite = keyedWrite<PhoneConfirmInput, PhoneConfirmOutput>(
    props.api,
    'POST',
    (b) => `/api/customers/${b.id}/phone-verifications/${b.challengeId}/confirm`,
  );

  async function handleSend(): Promise<void> {
    const trimmed = phone().trim();
    if (!trimmed) return;
    setError(undefined);
    try {
      const out = await startWrite.send({
        id: props.customerId,
        branch: props.branch,
        phone: trimmed,
      });
      setChallengeId(out.challengeId);
      setVerified(false);
    } catch (err: unknown) {
      setError(refusalText(err));
    }
  }

  async function handleConfirm(): Promise<void> {
    const cid = challengeId();
    if (!cid) return;
    setError(undefined);
    try {
      const out = await confirmWrite.send({
        id: props.customerId,
        challengeId: cid,
        branch: props.branch,
        code: code(),
      });
      if (out.verified) {
        setVerified(true);
        props.onVerified();
      }
    } catch (err: unknown) {
      if (err instanceof ApiError && (err.code === 'tooManyAttempts' || err.code === 'codeExpired')) {
        setCode('');
        if (codeInputRef) codeInputRef.value = '';
      }
      setError(refusalText(err));
    }
  }

  return (
    <div>
      <label class="desk-field">
        <span>{t('phone')}</span>
        <input
          type="tel"
          name="phone"
          value={phone()}
          onInput={(e) => setPhone(e.currentTarget.value)}
        />
      </label>
      <div class="desk-actions">
        <button type="button" name="sendCode" onClick={() => void handleSend()}>
          {t('sendCode')}
        </button>
      </div>
      <Show when={challengeId()}>
        <label class="desk-field">
          <span>{t('code')}</span>
          <input
            ref={codeInputRef}
            type="text"
            name="code"
            autocomplete="one-time-code"
            value={code()}
            onInput={(e) => setCode(e.currentTarget.value)}
          />
        </label>
        <div class="desk-actions">
          <button type="button" name="confirmCode" onClick={() => void handleConfirm()}>
            {t('confirmCode')}
          </button>
        </div>
      </Show>
      <Show when={verified()}>
        <p class="desk-notice" role="status">
          {t('verified')}
        </p>
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
