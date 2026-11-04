// APP-7 (#998, #5958) C14 sign-in (design customer-app.md §4.5): Identity's OTP and phone+password through OR-3's BFF.
// Two doors: OTP (default, unchanged: request -> verify) and password (POST /api/auth/login).
// Only a `200` navigates to `/app` (or `next`).
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
  const [mode, setMode] = createSignal<'otp' | 'password'>('otp');

  // OTP mode state
  const [challenge, setChallenge] = createSignal<string | undefined>();
  const [error, setError] = createSignal('');
  const [phoneError, setPhoneError] = createSignal('');
  const [codeError, setCodeError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  const [sending, setSending] = createSignal(false);
  const [hasCode, setHasCode] = createSignal(false);
  const [otpPhone, setOtpPhone] = createSignal('');
  const [otpCode, setOtpCode] = createSignal('');
  let phoneInput: HTMLInputElement | undefined;
  let codeInput: HTMLInputElement | undefined;
  let requesting: Promise<void> | undefined;

  // Password mode state
  const [pwdPhone, setPwdPhone] = createSignal('');
  const [pwdPhoneError, setPwdPhoneError] = createSignal('');
  const [pwdPasswordError, setPwdPasswordError] = createSignal('');
  const [pwdPending, setPwdPending] = createSignal(false);
  let pwdPhoneInput: HTMLInputElement | undefined;
  let pwdPasswordInput: HTMLInputElement | undefined;

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

  const showPasswordError = (err: unknown) => {
    const fe = fieldError(err, { phone: tr('phone'), password: tr('password') });
    let target = fe.field;
    if (!target && err instanceof ApiError && !isServerFailure(err)) {
      if (err.code === 'invalidCredentials') target = 'password';
      else if (PHONE_CODES.has(err.code)) target = 'phone';
    }
    if (target === 'password') {
      setPwdPasswordError(fe.message);
      if (pwdPasswordInput) pwdPasswordInput.value = '';
    } else if (target === 'phone') {
      setPwdPhoneError(fe.message);
    } else {
      setError(fe.message);
    }
  };

  const requestCode = (): Promise<void> => {
    if (requesting) return requesting;
    const rawPhone = (phoneInput?.value ?? otpPhone()).trim();
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
    const code = (codeInput?.value ?? otpCode()).trim();
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

  const submitPassword = async (e: Event) => {
    e.preventDefault();
    if (pwdPending()) return;
    const rawPhone = (pwdPhoneInput?.value ?? pwdPhone()).trim();
    const rawPassword = pwdPasswordInput?.value ?? '';
    let hasErr = false;
    if (!rawPhone) {
      setPwdPhoneError(t('input.code.phoneRequired'));
      hasErr = true;
    }
    if (!rawPassword) {
      setPwdPasswordError(tr('passwordRequired'));
      hasErr = true;
    }
    if (hasErr) return;

    setPwdPending(true);
    setError('');
    setPwdPhoneError('');
    setPwdPasswordError('');

    try {
      await api.post('/api/auth/login', {
        identifier: e164(rawPhone),
        password: rawPassword,
      });
      props.navigate(next() ?? '/app');
    } catch (err: unknown) {
      showPasswordError(err);
    } finally {
      setPwdPending(false);
    }
  };

  return (
    <section class="xc-sign-in">
      <h2>{tr('signInTitle')}</h2>
      <div class="xc-sign-in-mode" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={mode() === 'otp'}
          class={mode() === 'otp' ? 'xc-sign-in-tab active' : 'xc-sign-in-tab'}
          onClick={() => {
            setError('');
            setMode('otp');
          }}
        >
          {tr('modeOtp')}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode() === 'password'}
          class={mode() === 'password' ? 'xc-sign-in-tab active' : 'xc-sign-in-tab'}
          onClick={() => {
            setError('');
            setMode('password');
          }}
        >
          {tr('modePassword')}
        </button>
      </div>
      <Show when={mode() === 'otp'}>
        <form class="xc-sign-in-form" onSubmit={submit}>
          <label>
            {tr('phone')}
            <input
              ref={phoneInput}
              type="tel"
              name="phone"
              autocomplete="tel"
              inputmode="tel"
              value={otpPhone()}
              aria-invalid={phoneError() ? 'true' : undefined}
              aria-describedby={phoneError() ? 'phone-error' : undefined}
              onInput={(e) => {
                setOtpPhone(e.currentTarget.value);
                setPhoneError('');
              }}
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
              value={otpCode()}
              aria-invalid={codeError() ? 'true' : undefined}
              aria-describedby={codeError() ? 'code-error' : undefined}
              onInput={(e) => {
                setOtpCode(e.currentTarget.value);
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
      </Show>
      <Show when={mode() === 'password'}>
        <form class="xc-sign-in-form" onSubmit={submitPassword}>
          <label>
            {tr('phone')}
            <input
              ref={pwdPhoneInput}
              type="tel"
              name="phone"
              autocomplete="tel"
              inputmode="tel"
              value={pwdPhone()}
              aria-invalid={pwdPhoneError() ? 'true' : undefined}
              aria-describedby={pwdPhoneError() ? 'pwd-phone-error' : undefined}
              onInput={(e) => {
                setPwdPhone(e.currentTarget.value);
                setPwdPhoneError('');
              }}
            />
            <Show when={pwdPhoneError()}>
              <span id="pwd-phone-error" role="alert" class="xc-sign-in-error" data-field-error="phone">
                {pwdPhoneError()}
              </span>
            </Show>
          </label>
          <label>
            {tr('password')}
            <input
              ref={pwdPasswordInput}
              type="password"
              name="password"
              autocomplete="current-password"
              aria-invalid={pwdPasswordError() ? 'true' : undefined}
              aria-describedby={pwdPasswordError() ? 'pwd-password-error' : undefined}
              onInput={() => setPwdPasswordError('')}
            />
            <Show when={pwdPasswordError()}>
              <span id="pwd-password-error" role="alert" class="xc-sign-in-error" data-field-error="password">
                {pwdPasswordError()}
              </span>
            </Show>
          </label>
          <button type="submit" disabled={pwdPending()}>
            {tr('signInTitle')}
          </button>
        </form>
      </Show>
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
