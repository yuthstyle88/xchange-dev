// APP-7 (#998) C14 sign-in (design customer-app.md §4.5): Identity's OTP through OR-3's BFF. One form: with the code
// empty, submitting sends the phone number to `POST /api/auth/otp/request`; with a code typed, it sends it to
// `POST /api/auth/otp/verify` (after the code request has answered), and only a `200` from verify navigates to `/app`.
// The server claims this browser's anonymous drafts at verify (P6). No demo login and no fixed code.
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { screenT, t } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure } from '../../../shared/i18n/worded-error';
import { safeNext } from '../../login/home';
import * as table from './i18n';
import './sign-in.css';

const tr = screenT('customer.sign-in', table);
const PHONE_CODES = new Set(['invalidPhone', 'phoneRequired', 'tooManyAttempts', 'invalidCredentials']);
const CODE_CODES = new Set(['invalidCode', 'codeExpired']);

/** A Thai number typed with its leading 0 becomes E.164 (+66…); anything else is sent as typed. */
export function e164(phone: string): string {
  const digits = phone.replace(/[\s-]/g, '');
  return /^0\d{8,9}$/.test(digits) ? `+66${digits.slice(1)}` : digits;
}

export function nextTarget(search?: string): string | null {
  const safe = safeNext(new URLSearchParams(search ?? window.location.search).get('next'));
  if (
    safe &&
    (safe === '/app' || safe.startsWith('/app/') || safe.startsWith('/app?')) &&
    safe !== '/app/sign-in' &&
    !safe.startsWith('/app/sign-in/') &&
    !safe.startsWith('/app/sign-in?') &&
    !safe.startsWith('/app/sign-in#')
  ) {
    return safe;
  }
  return null;
}

export function SignIn(props: {
  navigate: (url: string) => void;
  fetch?: typeof fetch;
  search?: string;
}): JSX.Element {
  const api = new ApiClient(props.fetch);
  const next = () => nextTarget(props.search);
  const [challenge, setChallenge] = createSignal<string | undefined>();
  const [error, setError] = createSignal('');
  const [phoneError, setPhoneError] = createSignal('');
  const [codeError, setCodeError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  const [sending, setSending] = createSignal(false);
  const [hasCode, setHasCode] = createSignal(false);
  let phoneInput: HTMLInputElement | undefined;
  let codeInput: HTMLInputElement | undefined;
  let requesting: Promise<void> | undefined;

  const showError = (err: unknown, defaultCodeTarget: 'phone' | 'code') => {
    const fe = fieldError(err, { phone: tr('phone'), code: tr('code') });
    let target = fe.field;
    if (!target && err instanceof ApiError && !isServerFailure(err)) {
      if (CODE_CODES.has(err.code)) target = 'code';
      else if (PHONE_CODES.has(err.code)) target = defaultCodeTarget === 'code' && err.code === 'tooManyAttempts' ? 'code' : 'phone';
    }
    if (target === 'phone') {
      setPhoneError(fe.message);
    } else if (target === 'code') {
      setCodeError(fe.message);
    } else {
      setError(fe.message);
    }
  };

  const requestCode = (): Promise<void> => {
    if (requesting) return requesting;
    const rawPhone = (phoneInput?.value ?? '').trim();
    if (!rawPhone) {
      setPhoneError(t('input.code.phoneRequired'));
      return Promise.resolve();
    }
    setError('');
    setPhoneError('');
    setCodeError('');
    setSending(true);
    const run = (async () => {
      try {
        const json = await api.post<
          { destination: string; channel: string },
          { challengeId?: string }
        >('/api/auth/otp/request', {
          destination: e164(rawPhone),
          channel: 'sms',
        });
        if (json?.challengeId) {
          setChallenge(json.challengeId);
        } else {
          setError(tr('requestFailed'));
        }
      } catch (err: unknown) {
        showError(err, 'phone');
      } finally {
        requesting = undefined;
        setSending(false);
      }
    })();
    requesting = run;
    return run;
  };

  const verify = async (code: string) => {
    if (pending()) return;
    setPending(true);
    try {
      await requesting;
      const challengeId = challenge();
      if (!challengeId) return;
      setError('');
      setPhoneError('');
      setCodeError('');
      await api.post('/api/auth/otp/verify', { challengeId, code });
      props.navigate(next() ?? '/app');
    } catch (err: unknown) {
      showError(err, 'code');
    } finally {
      setPending(false);
    }
  };

  const submit = (e: Event) => {
    e.preventDefault();
    const code = (codeInput?.value ?? '').trim();
    if (challenge() || requesting) {
      if (!code) {
        setCodeError(tr('codeRequired'));
        return;
      }
      void verify(code);
    } else {
      void requestCode();
    }
  };

  return (
    <section class="xc-sign-in">
      <h2>{tr('signInTitle')}</h2>
      <form class="xc-sign-in-form" onSubmit={submit}>
        <label>
          {tr('phone')}
          <input
            ref={phoneInput}
            type="tel"
            name="phone"
            autocomplete="tel"
            inputmode="tel"
            aria-invalid={phoneError() ? 'true' : undefined}
            aria-describedby={phoneError() ? 'phone-error' : undefined}
            onInput={() => setPhoneError('')}
          />
          <Show when={phoneError()}>
            <span id="phone-error" role="alert" class="xc-sign-in-error" data-field-error="phone">
              {phoneError()}
            </span>
          </Show>
        </label>
        <Show when={challenge()}>
          <p>{tr('codeSent')}</p>
        </Show>
        <label>
          {tr('code')}
          <input
            ref={codeInput}
            type="text"
            name="code"
            autocomplete="one-time-code"
            inputmode="numeric"
            aria-invalid={codeError() ? 'true' : undefined}
            aria-describedby={codeError() ? 'code-error' : undefined}
            onInput={(e) => {
              setHasCode(e.currentTarget.value.trim().length > 0);
              setCodeError('');
            }}
          />
          <Show when={codeError()}>
            <span id="code-error" role="alert" class="xc-sign-in-error" data-field-error="code">
              {codeError()}
            </span>
          </Show>
        </label>
        <button type="submit" disabled={pending() || (sending() && !hasCode())}>
          {challenge() ? tr('verify') : tr('requestCode')}
        </button>
        <Show when={challenge()}>
          <button type="button" class="xc-sign-in-link" disabled={sending()} onClick={() => void requestCode()}>
            {tr('resendCode')}
          </button>
        </Show>
      </form>
      <Show when={error()}>
        <p role="alert" class="xc-sign-in-error">
          {error()}
        </p>
      </Show>
      <p class="xc-sign-in-switch">
        {tr('noAccount')}{' '}
        <a href={next() ? `/app/sign-up?next=${encodeURIComponent(next()!)}` : '/app/sign-up'}>
          {tr('toSignUp')}
        </a>
      </p>
    </section>
  );
}
