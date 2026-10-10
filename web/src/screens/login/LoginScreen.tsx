import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { ApiError } from '../../shared/api/client';
import { AuthLayout } from '../../shared/ui/AuthLayout';
import { PasswordField } from '../../shared/ui/PasswordField';
import { TextField } from '../../shared/ui/TextField';
import { useForm } from '../../shared/ui/form';
import { screenT } from '../../shared/i18n';
import type { AuthApi } from './auth-api';
import { homeOf } from './home';
import * as loginI18n from './i18n';

const t = screenT('login', loginI18n);

function toErrorKey(err: unknown): keyof typeof loginI18n.th {
  if (err instanceof ApiError) {
    if (err.code === 'inviteRefused' && err.reason) {
      const reasonKey = `err_inviteRefused_${err.reason}` as keyof typeof loginI18n.th;
      if (reasonKey in loginI18n.th) {
        return reasonKey;
      }
      return 'err_inviteRefused';
    }
    const key = `err_${err.code}` as keyof typeof loginI18n.th;
    if (key in loginI18n.th) {
      return key;
    }
    return 'errUnknown';
  }
  return 'errNetwork';
}

export function LoginScreen(props: {
  auth: AuthApi;
  navigate: (url: string) => void;
}): JSX.Element {
  const form = useForm();
  let formEl: HTMLFormElement | undefined;
  const [pending, setPending] = createSignal(false);
  const [showInvite, setShowInvite] = createSignal(false);
  const [errorKey, setErrorKey] = createSignal<keyof typeof loginI18n.th | undefined>(undefined);

  onMount(async () => {
    try {
      const res = await props.auth.session();
      if (res.user) {
        props.navigate(homeOf(res.user));
      }
    } catch (err) {
      setErrorKey(toErrorKey(err));
    }
  });

  const onSubmit = form.submit(async () => {
    if (pending()) return;
    if (!formEl) return;

    const data = new FormData(formEl);
    const identifier = String(data.get('identifier') ?? '');
    const password = String(data.get('password') ?? '');
    const inviteCode = String(data.get('inviteCode') ?? '');

    setPending(true);
    setErrorKey(undefined);

    try {
      if (showInvite()) {
        const res = await props.auth.claim({ identifier, password, inviteCode });
        props.navigate(homeOf(res.user));
      } else {
        const res = await props.auth.login({ identifier, password });
        props.navigate(homeOf(res.user));
      }
    } catch (err) {
      if (err instanceof ApiError && err.code === 'signIn') {
        setShowInvite(true);
      }
      setErrorKey(toErrorKey(err));
    } finally {
      setPending(false);
    }
  });

  const handleSubmit = (e?: Event) => {
    if (pending()) {
      if (e && typeof e.preventDefault === 'function') {
        e.preventDefault();
      }
      return;
    }
    return onSubmit(e);
  };

  const message = () => {
    const k = errorKey();
    return k ? t(k) : '';
  };

  return (
    <AuthLayout title={t('title')}>
      <form ref={formEl} onSubmit={handleSubmit}>
        <TextField
          field={form.field('identifier')}
          label={t('identifier')}
          autocomplete="username"
          required
          maxLength={254}
        />
        <PasswordField
          field={form.field('password')}
          label={t('password')}
          autocomplete="current-password"
          required
          maxLength={128}
        />
        <Show when={showInvite()}>
          <TextField
            field={form.field('inviteCode')}
            label={t('inviteCode')}
            required={showInvite()}
          />
        </Show>
        <div>
          <button
            type="button"
            onClick={() => setShowInvite(true)}
          >
            {t('haveInvite')}
          </button>
        </div>
        <div>
          <button type="submit" disabled={pending()}>
            {t('submit')}
          </button>
        </div>
        <p id="message" role="alert">
          {message()}
        </p>
      </form>
    </AuthLayout>
  );
}
