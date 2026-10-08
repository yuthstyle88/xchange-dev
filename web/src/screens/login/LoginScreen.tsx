import { createSignal, onMount, type JSX } from 'solid-js';
import { ApiError } from '../../shared/api/client';
import { AppShell } from '../../shared/ui/AppShell';
import { PasswordField } from '../../shared/ui/PasswordField';
import { TextField } from '../../shared/ui/TextField';
import { useForm } from '../../shared/ui/form';
import { screenT } from '../../shared/i18n';
import type { AuthApi } from './auth-api';
import { homeOf } from './home';
import * as loginI18n from './i18n';

const t = screenT('login', loginI18n);

export function LoginScreen(props: {
  auth: AuthApi;
  navigate: (url: string) => void;
}): JSX.Element {
  const form = useForm();
  let formEl: HTMLFormElement | undefined;
  const [pending, setPending] = createSignal(false);
  const [statusError, setStatusError] = createSignal<string | undefined>(undefined);
  const [submitError, setSubmitError] = createSignal<string | undefined>(undefined);

  onMount(async () => {
    try {
      const res = await props.auth.status();
      if (res.user) {
        props.navigate(homeOf(res.user));
      }
    } catch (err) {
      const msg =
        err instanceof ApiError
          ? err.message
          : err instanceof Error
            ? err.message
            : String(err);
      setStatusError(msg);
    }
  });

  const onSubmit = form.submit(async () => {
    if (pending()) return;
    if (!formEl) return;

    const data = new FormData(formEl);
    const username = String(data.get('username') ?? '');
    const password = String(data.get('password') ?? '');

    setPending(true);
    setStatusError(undefined);
    setSubmitError(undefined);

    try {
      const res = await props.auth.login({ username, password });
      if (res.mustChange) {
        props.navigate('/staff');
      } else {
        props.navigate(homeOf(res.user));
      }
    } catch (err) {
      if (err instanceof ApiError) {
        throw err;
      }
      setSubmitError(err instanceof Error ? err.message : String(err));
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
    setStatusError(undefined);
    return onSubmit(e);
  };

  const message = () => statusError() ?? submitError() ?? form.formError() ?? '';

  return (
    <AppShell title={t('title')}>
      <div
        style={{
          'max-width': '420px',
          margin: '40px auto',
          padding: '32px',
          background: '#fff',
          'border-radius': '8px',
          'box-shadow': '0 2px 8px rgba(0,0,0,0.08)',
          'font-family': 'Tahoma, sans-serif',
        }}
      >
        <div
          style={{
            'font-size': '12px',
            'letter-spacing': '1px',
            color: '#6b8090',
            'margin-bottom': '6px',
          }}
        >
          XCHANGE × T.K.
        </div>
        <form ref={formEl} onSubmit={handleSubmit}>
          <TextField
            field={form.field('username')}
            label={t('username')}
            autocomplete="username"
            required
            pattern="[a-zA-Z0-9._\-]{3,40}"
            maxLength={40}
          />
          <PasswordField
            field={form.field('password')}
            label={t('password')}
            autocomplete="current-password"
            required
            maxLength={128}
          />
          <div style={{ 'margin-top': '16px' }}>
            <button
              type="submit"
              disabled={pending()}
              style={{
                background: '#183044',
                color: '#fff',
                border: 'none',
                'border-radius': '4px',
                padding: '10px 16px',
                cursor: pending() ? 'not-allowed' : 'pointer',
                opacity: pending() ? '0.7' : '1',
              }}
            >
              {t('submit')}
            </button>
          </div>
          <p
            id="message"
            role="alert"
            style={{
              color: '#a5003c',
              'font-size': '13px',
              'min-height': '18px',
              margin: '12px 0 0',
            }}
          >
            {message()}
          </p>
        </form>
      </div>
    </AppShell>
  );
}
