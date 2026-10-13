// APP-7 (#998) C14 sign-in (design customer-app.md §4.5): Identity's OTP through OR-3's BFF. One form: with the code
// empty, submitting sends the phone number to `POST /api/auth/otp/request`; with a code typed, it sends it to
// `POST /api/auth/otp/verify` (after the code request has answered), and only a `200` from verify navigates to `/app`.
// The server claims this browser's anonymous drafts at verify (P6). No demo login and no fixed code.
import { createSignal, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';
import './sign-in.css';

const tr = screenT('customer.sign-in', table);

/** A Thai number typed with its leading 0 becomes E.164 (+66…); anything else is sent as typed. */
export function e164(phone: string): string {
  const digits = phone.replace(/[\s-]/g, '');
  return /^0\d{8,9}$/.test(digits) ? `+66${digits.slice(1)}` : digits;
}

async function postJson(fetchFn: typeof fetch, url: string, body: unknown): Promise<Response> {
  return fetchFn(url, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export function SignIn(props: { navigate: (url: string) => void; fetch?: typeof fetch }): JSX.Element {
  const fetchFn: typeof fetch = (input, init) => (props.fetch ?? fetch)(input, init);
  const [challenge, setChallenge] = createSignal<string | undefined>();
  const [error, setError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  let phoneInput: HTMLInputElement | undefined;
  let codeInput: HTMLInputElement | undefined;
  let requesting: Promise<void> | undefined;

  const requestCode = (): Promise<void> => {
    setError('');
    requesting = (async () => {
      try {
        const res = await postJson(fetchFn, '/api/auth/otp/request', {
          destination: e164(phoneInput?.value ?? ''),
          channel: 'sms',
        });
        const json = (await res.json().catch(() => ({}))) as { challengeId?: string };
        if (res.status === 200 && json.challengeId) {
          setChallenge(json.challengeId);
        } else {
          setError(tr('requestFailed'));
        }
      } catch {
        setError(tr('requestFailed'));
      }
    })();
    return requesting;
  };

  const verify = async (code: string) => {
    if (pending()) return;
    setPending(true);
    try {
      await requesting;
      const challengeId = challenge();
      if (!challengeId) return;
      setError('');
      const res = await postJson(fetchFn, '/api/auth/otp/verify', { challengeId, code });
      if (res.status === 200) {
        props.navigate('/app');
        return;
      }
      setError(tr('signInFailed'));
    } catch {
      setError(tr('signInFailed'));
    } finally {
      setPending(false);
    }
  };

  const submit = (e: Event) => {
    e.preventDefault();
    const code = (codeInput?.value ?? '').trim();
    if (code) {
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
          <input ref={phoneInput} type="tel" name="phone" autocomplete="tel" inputmode="tel" />
        </label>
        <Show when={challenge()}>
          <p>{tr('codeSent')}</p>
        </Show>
        <label>
          {tr('code')}
          <input ref={codeInput} type="text" name="code" autocomplete="one-time-code" inputmode="numeric" />
        </label>
        <button type="submit" disabled={pending()}>
          {challenge() ? tr('verify') : tr('requestCode')}
        </button>
        <Show when={challenge()}>
          <button type="button" class="xc-sign-in-link" onClick={() => void requestCode()}>
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
        {tr('noAccount')} <a href="/app/sign-up">{tr('toSignUp')}</a>
      </p>
    </section>
  );
}
