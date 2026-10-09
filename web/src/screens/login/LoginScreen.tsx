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
  let idEl: HTMLInputElement | undefined;
  let phoneEl: HTMLInputElement | undefined;
  let emailEl: HTMLInputElement | undefined;

  const [pending, setPending] = createSignal(false);
  const [showInvite, setShowInvite] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [registered, setRegistered] = createSignal(false);
  const [errorKey, setErrorKey] = createSignal<keyof typeof loginI18n.th | undefined>(undefined);

  const [prefilledId, setPrefilledId] = createSignal('');

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
    const phone = String(data.get('phone') ?? '');
    const email = String(data.get('email') ?? '');

    setPending(true);
    setErrorKey(undefined);

    try {
      if (showInvite()) {
        if (creating()) {
          if (!registered()) {
            const emailParam = email.trim() || undefined;
            await props.auth.register({
              phone,
              password,
              email: emailParam,
            });
            setRegistered(true);
          }
          const res = await props.auth.claim({
            identifier: phone,
            password,
            inviteCode,
          });
          props.navigate(homeOf(res.user));
        } else {
          const res = await props.auth.claim({
            identifier,
            password,
            inviteCode,
          });
          props.navigate(homeOf(res.user));
        }
      } else {
        const res = await props.auth.login({ identifier, password });
        props.navigate(homeOf(res.user));
      }
    } catch (err) {
      if (err instanceof ApiError && err.code === 'signIn') {
        setShowInvite(true);
      }
      if (err instanceof ApiError && err.code === 'alreadyRegistered') {
        if (phoneEl) {
          phoneEl.required = false;
          phoneEl.disabled = true;
        }
        if (emailEl) {
          emailEl.disabled = true;
        }
        setPrefilledId(phone);
        setCreating(false);
        setRegistered(false);
        if (idEl) {
          idEl.disabled = false;
          idEl.value = phone;
        }
        setErrorKey('err_alreadyRegistered');
        return;
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

  const idField = () => {
    const f = form.field('identifier');
    return {
      ...f,
      ref: (el: HTMLInputElement) => {
        idEl = el;
        if (prefilledId()) {
          el.value = prefilledId();
        }
        f.ref(el);
      },
    };
  };

  const phoneField = () => {
    const f = form.field('phone');
    return {
      ...f,
      ref: (el: HTMLInputElement) => {
        phoneEl = el;
        f.ref(el);
      },
    };
  };

  const emailField = () => {
    const f = form.field('email');
    return {
      ...f,
      ref: (el: HTMLInputElement) => {
        emailEl = el;
        f.ref(el);
      },
    };
  };

  return (
    <AuthLayout title={t('title')}>
      <form ref={formEl} onSubmit={handleSubmit}>
        <Show when={!creating()}>
          <TextField
            field={idField()}
            label={t('identifier')}
            autocomplete="username"
            required={!creating()}
            maxLength={254}
          />
        </Show>
        <Show when={creating()}>
          <TextField
            field={phoneField()}
            label={t('phone')}
            autocomplete="tel"
            required={creating()}
          />
          <TextField
            field={emailField()}
            label={t('email')}
            type="email"
            autocomplete="email"
            required={false}
          />
        </Show>
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
          <Show when={!creating()}>
            <div>
              <button
                type="button"
                onClick={() => {
                  setCreating(true);
                  if (idEl) {
                    idEl.disabled = true;
                  }
                }}
              >
                {t('createAccount')}
              </button>
            </div>
          </Show>
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
