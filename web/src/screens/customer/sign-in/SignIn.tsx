// APP-7 (#998, #5958) C14 sign-in (design customer-app.md §4.5): Identity's OTP and phone+password through OR-3's BFF.
// Two doors: OTP (default, unchanged: request -> verify) and password (POST /api/auth/login).
// Only a `200` navigates to `/app` (or `next`).
// The server claims this browser's anonymous drafts at verify (P6). No demo login and no fixed code.
import { createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { screenT, t } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure } from '../../../shared/i18n/worded-error';
import { useForm } from '../../../shared/ui/form';
import { PasswordField } from '../../../shared/ui/PasswordField';
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

export type OtpChannelChoice = 'email' | 'sms';

const DEFAULT_CHANNELS: readonly OtpChannelChoice[] = ['email', 'sms'];

export function SignIn(props: {
  navigate: (url: string) => void;
  fetch?: typeof fetch;
  search?: string;
  channels?: readonly OtpChannelChoice[];
  channelSource?: 'default' | 'set';
}): JSX.Element {
  const api = new ApiClient(props.fetch);
  const next = () => nextTarget(props.search);
  const [mode, setMode] = createSignal<'otp' | 'password'>('otp');
  const [loadedChannels, setLoadedChannels] = createSignal<readonly OtpChannelChoice[] | undefined>();
  const [loadedSource, setLoadedSource] = createSignal<'default' | 'set' | undefined>();
  const allowedChannels = () => {
    const raw = props.channels ?? loadedChannels() ?? DEFAULT_CHANNELS;
    const list = raw.filter((c): c is OtpChannelChoice => c === 'email' || c === 'sms');
    return list.length > 0 ? list : DEFAULT_CHANNELS;
  };
  const channelSource = () => props.channelSource ?? loadedSource() ?? 'default';
  const [pickedChannel, setPickedChannel] = createSignal<OtpChannelChoice | undefined>();
  const activeChannel = (): OtpChannelChoice => {
    const allowed = allowedChannels();
    const picked = pickedChannel();
    return picked && allowed.includes(picked) ? picked : allowed[0]!;
  };

  // OTP mode state
  const [challenge, setChallenge] = createSignal<string | undefined>();
  const [error, setError] = createSignal('');
  const [phoneError, setPhoneError] = createSignal('');
  const [emailError, setEmailError] = createSignal('');
  const [codeError, setCodeError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  const [sending, setSending] = createSignal(false);
  const [hasCode, setHasCode] = createSignal(false);
  const [otpPhone, setOtpPhone] = createSignal('');
  const [otpEmail, setOtpEmail] = createSignal('');
  const [otpCode, setOtpCode] = createSignal('');
  let phoneInput: HTMLInputElement | undefined;
  let emailInput: HTMLInputElement | undefined;
  let codeInput: HTMLInputElement | undefined;
  let requesting: Promise<void> | undefined;
  // Bumped when the customer switches channel: a code request started before the switch is stale, and its answer
  // (challenge, error) and its `finally` must not touch the new channel's state (review of PR 6272 at 18c173c).
  let requestGeneration = 0;
  let disposed = false;
  onCleanup(() => {
    disposed = true;
  });
  onMount(() => {
    if (props.channels) return;
    queueMicrotask(() => {
      if (disposed || mode() !== 'otp' || requesting || challenge() || phoneError() || emailError()) return;
      void api
        .publicGet<{ channels?: unknown; source?: unknown }>('/api/auth/otp/channels')
        .then((res) => {
          if (disposed) return;
          const valid = Array.isArray(res?.channels)
            ? res.channels.filter((c): c is OtpChannelChoice => c === 'email' || c === 'sms')
            : [];
          if (valid.length > 0) {
            setLoadedChannels(valid);
            setLoadedSource(res?.source === 'set' ? 'set' : 'default');
          }
        })
        .catch(() => {});
    });
  });

  const selectChannel = (ch: OtpChannelChoice) => {
    setError('');
    setPhoneError('');
    setEmailError('');
    setCodeError('');
    if (activeChannel() !== ch) {
      requestGeneration += 1;
      requesting = undefined;
      setSending(false);
      setChallenge(undefined);
      setOtpCode('');
      setHasCode(false);
      if (codeInput) codeInput.value = '';
    }
    setPickedChannel(ch);
  };

  // Password mode state
  const form = useForm();
  const passwordField = form.field('password');
  const [pwdPhone, setPwdPhone] = createSignal('');
  const [pwdPhoneError, setPwdPhoneError] = createSignal('');
  const [pwdPasswordError, setPwdPasswordError] = createSignal('');
  const [pwdPending, setPwdPending] = createSignal(false);
  let pwdPhoneInput: HTMLInputElement | undefined;
  let pwdPasswordInput: HTMLInputElement | undefined;

  const passwordBinding = {
    ...passwordField,
    invalid: () => passwordField.invalid() || Boolean(pwdPasswordError()),
    error: () => pwdPasswordError() || passwordField.error(),
    ref: (el: HTMLInputElement) => {
      pwdPasswordInput = el;
      passwordField.ref(el);
      el.addEventListener('input', () => setPwdPasswordError(''));
    },
  };

  const showError = (err: unknown, defaultCodeTarget: 'phone' | 'code') => {
    const codeLabel = activeChannel() === 'email' ? tr('codeEmail') : tr('code');
    const fe = fieldError(err, { phone: tr('phone'), email: tr('email'), code: codeLabel });
    let target = fe.field;
    if (!target && err instanceof ApiError && !isServerFailure(err)) {
      if (CODE_CODES.has(err.code)) target = 'code';
      else if (PHONE_CODES.has(err.code)) target = defaultCodeTarget === 'code' && err.code === 'tooManyAttempts' ? 'code' : activeChannel() === 'email' ? 'email' : 'phone';
    }
    if (target === 'phone') {
      setPhoneError(fe.message);
    } else if (target === 'email') {
      setEmailError(fe.message);
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
    const ch = activeChannel();
    const rawEmail = (emailInput?.value ?? otpEmail()).trim();
    const rawPhone = (phoneInput?.value ?? otpPhone()).trim();
    if (ch === 'email') {
      if (!rawEmail) {
        setEmailError(tr('emailRequired'));
        return Promise.resolve();
      }
    } else if (!rawPhone) {
      setPhoneError(t('input.code.phoneRequired'));
      return Promise.resolve();
    }
    const destination = ch === 'email' ? rawEmail : e164(rawPhone);
    setError('');
    setPhoneError('');
    setEmailError('');
    setCodeError('');
    setSending(true);
    const generation = requestGeneration;
    const current = () => !disposed && generation === requestGeneration;
    const run = (async () => {
      try {
        const json = await api.post<
          { destination: string; channel: string },
          { challengeId?: string }
        >('/api/auth/otp/request', {
          destination,
          channel: ch,
        });
        if (!current()) return;
        if (json?.challengeId) {
          setChallenge(json.challengeId);
        } else {
          setError(tr('requestFailed'));
        }
      } catch (err: unknown) {
        if (current()) showError(err, 'phone');
      } finally {
        if (current()) {
          requesting = undefined;
          setSending(false);
        }
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
      setEmailError('');
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
          <Show when={allowedChannels().length > 1}>
            <div class="xc-sign-in-mode" role="radiogroup" data-source={channelSource()}>
              <For each={allowedChannels()}>
                {(ch) => (
                  <button
                    type="button"
                    role="radio"
                    data-channel={ch}
                    aria-checked={activeChannel() === ch}
                    class={activeChannel() === ch ? 'xc-sign-in-tab active' : 'xc-sign-in-tab'}
                    onClick={() => selectChannel(ch)}
                  >
                    {ch === 'email' ? tr('channelEmail') : tr('channelSms')}
                  </button>
                )}
              </For>
              <Show when={channelSource() === 'default'}>
                <span class="xc-sign-in-switch" data-default-note="true">
                  {tr('channelDefaultNote')}
                </span>
              </Show>
            </div>
          </Show>
          <Show when={activeChannel() === 'email'}>
            <label>
              {tr('email')}
              <input
                ref={emailInput}
                type="email"
                name="email"
                autocomplete="email"
                inputmode="email"
                value={otpEmail()}
                aria-invalid={emailError() ? 'true' : undefined}
                aria-describedby={emailError() ? 'email-error' : undefined}
                onInput={(e) => {
                  setOtpEmail(e.currentTarget.value);
                  setEmailError('');
                }}
              />
              <Show when={emailError()}>
                <span id="email-error" role="alert" class="xc-sign-in-error" data-field-error="email">
                  {emailError()}
                </span>
              </Show>
            </label>
          </Show>
          <Show when={allowedChannels().includes('sms') && (activeChannel() === 'sms' || pickedChannel() === undefined)}>
            <label hidden={activeChannel() !== 'sms'}>
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
                  if (pickedChannel() === undefined && allowedChannels().includes('sms')) {
                    setPickedChannel('sms');
                  }
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
          </Show>
          <Show when={challenge()}>
            <p>{activeChannel() === 'email' ? tr('codeSentEmail') : tr('codeSent')}</p>
          </Show>
          <label>
            {activeChannel() === 'email' ? tr('codeEmail') : tr('code')}
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
          <PasswordField
            field={passwordBinding}
            label={tr('password')}
            autocomplete="current-password"
            error={pwdPasswordError() || undefined}
          />
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
