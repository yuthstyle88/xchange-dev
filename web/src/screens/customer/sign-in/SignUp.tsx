// APP-7 (#998) C14 sign-up (design customer-app.md §4.5): CI-7's `customerRegister` (`POST /api/app/register`)
// registers the Identity user as a customer, records the TERMS consent in the page language, and signs them in; the
// server claims this browser's anonymous drafts at the same time (P6). The server checks every field; the screen
// shows its refusal, `consentTextMissing` in the page language when there is no published TERMS text in it.
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import { lang, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { useForm } from '../../../shared/ui/form';
import { PasswordField } from '../../../shared/ui/PasswordField';
import * as table from './i18n';
import './sign-in.css';

const tr = screenT('customer.sign-in', table);

export function SignUp(props: { navigate: (url: string) => void; fetch?: typeof fetch }): JSX.Element {
  const fetchFn: typeof fetch = (input, init) => (props.fetch ?? fetch)(input, init);
  const [error, setError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  let nameInput: HTMLInputElement | undefined;
  let identifierInput: HTMLInputElement | undefined;
  let passwordInput: HTMLInputElement | undefined;
  let marketingInput: HTMLInputElement | undefined;
  const passwordField = useForm().field('password');
  const passwordBinding = {
    ...passwordField,
    ref: (el: HTMLInputElement) => {
      passwordInput = el;
      passwordField.ref(el);
    },
  };

  const submit = async (e: Event) => {
    e.preventDefault();
    if (pending()) return;
    setPending(true);
    setError('');
    try {
      const res = await fetchFn('/api/app/register', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          identifier: (identifierInput?.value ?? '').trim(),
          password: passwordInput?.value ?? '',
          displayName: (nameInput?.value ?? '').trim(),
          lang: lang(),
          marketing: marketingInput?.checked ? ['SMS'] : [],
        }),
      });
      if (res.status === 200) {
        props.navigate('/app');
        return;
      }
      const json = (await res.json().catch(() => ({}))) as {
        error?: { code?: string; message?: string };
      };
      const code = json.error?.code ?? '';
      if (code === 'consentTextMissing') {
        setError(tr('consentTextMissing'));
      } else if (code === 'alreadyRegistered') {
        setError(tr('alreadyRegistered'));
      } else {
        throw new ApiError({
          status: res.status,
          code: code || (res.status >= 500 ? 'serverError' : 'httpError'),
          message: json.error?.message ?? '',
        });
      }
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setPending(false);
    }
  };

  return (
    <section class="xc-sign-in">
      <h2>{tr('signUpTitle')}</h2>
      <form class="xc-sign-in-form" onSubmit={submit}>
        <label>
          {tr('displayName')}
          <input ref={nameInput} type="text" name="displayName" autocomplete="name" />
        </label>
        <label>
          {tr('identifier')}
          <input ref={identifierInput} type="text" name="identifier" autocomplete="username" />
        </label>
        <PasswordField field={passwordBinding} label={tr('password')} autocomplete="new-password" />
        <label class="xc-sign-in-check">
          <input ref={marketingInput} type="checkbox" name="marketing" />
          {tr('marketing')}
        </label>
        <p class="xc-sign-in-terms">{tr('terms')}</p>
        <button type="submit" disabled={pending()}>
          {tr('submitSignUp')}
        </button>
      </form>
      <Show when={error()}>
        <p role="alert" class="xc-sign-in-error">
          {error()}
        </p>
      </Show>
      <p class="xc-sign-in-switch">
        {tr('haveAccount')} <a href="/app/sign-in">{tr('toSignIn')}</a>
      </p>
    </section>
  );
}
