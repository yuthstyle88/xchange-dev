// APP-7 (#998) C14 sign-up (design customer-app.md §4.5): CI-7's `customerRegister` (`POST /api/app/register`)
// registers the Identity user as a customer, records the TERMS consent in the page language, and signs them in; the
// server claims this browser's anonymous drafts at the same time (P6). The server checks every field; the screen
// shows its refusal, `consentTextMissing` in the page language when there is no published TERMS text in it.
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { lang, screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure } from '../../../shared/i18n/worded-error';
import { useForm } from '../../../shared/ui/form';
import { PasswordField } from '../../../shared/ui/PasswordField';
import { nextTarget } from './SignIn';
import { TermsText, type TermsState } from './TermsText';
import * as table from './i18n';
import './sign-in.css';

const tr = screenT('customer.sign-in', table);
const MAX_DISPLAY_NAME_LEN = 100;

export function SignUp(props: {
  navigate: (url: string) => void;
  fetch?: typeof fetch;
  search?: string;
}): JSX.Element {
  const client = new ApiClient(props.fetch);
  const next = () => nextTarget(props.search);
  const form = useForm();
  const [error, setError] = createSignal('');
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});
  const [pending, setPending] = createSignal(false);
  const [termsState, setTermsState] = createSignal<TermsState>('loading');
  let nameInput: HTMLInputElement | undefined;
  let identifierInput: HTMLInputElement | undefined;
  let passwordInput: HTMLInputElement | undefined;
  let marketingInput: HTMLInputElement | undefined;

  const nameField = form.field('displayName');
  const identifierField = form.field('identifier');
  const passwordField = form.field('password');

  const clearFieldError = (name: string) => {
    setFieldErrors((prev) => {
      if (!(name in prev)) return prev;
      const nextErrs = { ...prev };
      delete nextErrs[name];
      return nextErrs;
    });
  };

  const passwordBinding = {
    ...passwordField,
    invalid: () => passwordField.invalid() || Boolean(fieldErrors().password),
    error: () => fieldErrors().password ?? passwordField.error(),
    ref: (el: HTMLInputElement) => {
      passwordInput = el;
      passwordField.ref(el);
      el.addEventListener('input', () => clearFieldError('password'));
    },
  };

  const runSubmit = form.submit(async () => {
    setPending(true);
    setError('');
    setFieldErrors({});
    try {
      await client.post('/api/app/register', {
        identifier: (identifierInput?.value ?? '').trim(),
        password: passwordInput?.value ?? '',
        displayName: (nameInput?.value ?? '').trim(),
        lang: lang(),
        marketing: marketingInput?.checked ? ['SMS'] : [],
      });
      props.navigate(next() ?? '/app');
    } catch (err: unknown) {
      if (err instanceof ApiError && !isServerFailure(err)) {
        if (err.code === 'consentTextMissing') {
          setError(tr('consentTextMissing'));
          return;
        }
        if (err.code === 'alreadyRegistered') {
          setError(tr('alreadyRegistered'));
          return;
        }
      }
      const fe = fieldError(err, {
        displayName: tr('displayName'),
        identifier: tr('identifier'),
        password: tr('password'),
      });
      if (fe.field) {
        setFieldErrors({ [fe.field]: fe.message });
      } else {
        setError(fe.message);
      }
    } finally {
      setPending(false);
    }
  });

  const submit = (e: Event) => {
    e.preventDefault();
    if (pending() || termsState() !== 'ready') return;
    if (nameInput && nameInput.value.trim() === '') nameInput.value = '';
    if (identifierInput && identifierInput.value.trim() === '') identifierInput.value = '';
    void runSubmit(e);
  };

  const nameMsg = () => fieldErrors().displayName ?? (nameField.invalid() ? nameField.error() : undefined);
  const identifierMsg = () => fieldErrors().identifier ?? (identifierField.invalid() ? identifierField.error() : undefined);

  return (
    <section class="xc-sign-in">
      <h2>{tr('signUpTitle')}</h2>
      <form class="xc-sign-in-form" onSubmit={submit}>
        <label>
          {tr('displayName')}
          <input
            ref={(el) => {
              nameInput = el;
              nameField.ref(el);
            }}
            type="text"
            name="displayName"
            autocomplete="name"
            required
            maxLength={MAX_DISPLAY_NAME_LEN}
            aria-invalid={nameMsg() ? 'true' : undefined}
            aria-describedby={nameMsg() ? nameField.errorId : undefined}
            onInput={(e) => {
              if (e.currentTarget.value.length > MAX_DISPLAY_NAME_LEN) {
                e.currentTarget.value = e.currentTarget.value.slice(0, MAX_DISPLAY_NAME_LEN);
              }
              clearFieldError('displayName');
            }}
          />
          <Show when={nameMsg()}>
            <span id={nameField.errorId} role="alert" class="xc-sign-in-error" data-field-error="displayName">
              {nameMsg()}
            </span>
          </Show>
        </label>
        <label>
          {tr('identifier')}
          <input
            ref={(el) => {
              identifierInput = el;
              identifierField.ref(el);
            }}
            type="text"
            name="identifier"
            autocomplete="username"
            required
            aria-invalid={identifierMsg() ? 'true' : undefined}
            aria-describedby={identifierMsg() ? identifierField.errorId : undefined}
            onInput={() => clearFieldError('identifier')}
          />
          <Show when={identifierMsg()}>
            <span id={identifierField.errorId} role="alert" class="xc-sign-in-error" data-field-error="identifier">
              {identifierMsg()}
            </span>
          </Show>
        </label>
        <PasswordField field={passwordBinding} label={tr('password')} autocomplete="new-password" required />
        <label class="xc-sign-in-check">
          <input ref={marketingInput} type="checkbox" name="marketing" />
          {tr('marketing')}
        </label>
        <TermsText fetch={props.fetch} onState={setTermsState} />
        <button type="submit" disabled={pending() || termsState() !== 'ready'}>
          {tr('submitSignUp')}
        </button>
      </form>
      <Show when={error()}>
        <p role="alert" class="xc-sign-in-error">
          {error()}
        </p>
      </Show>
      <p class="xc-sign-in-switch">
        {tr('haveAccount')}{' '}
        <a href={next() ? `/app/sign-in?next=${encodeURIComponent(next()!)}` : '/app/sign-in'}>
          {tr('toSignIn')}
        </a>
      </p>
    </section>
  );
}
