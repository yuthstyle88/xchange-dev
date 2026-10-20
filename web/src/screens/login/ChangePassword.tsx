import { createSignal, type JSX } from 'solid-js';
import { ApiError } from '../../shared/api/client';
import { screenT } from '../../shared/i18n';
import { PasswordField } from '../../shared/ui/PasswordField';
import { useForm } from '../../shared/ui/form';
import type { PasswordApi } from './auth-api';
import * as loginI18n from './i18n';

const t = screenT('login', loginI18n);

function toChangeErrorKey(err: unknown): keyof typeof loginI18n.th {
  if (err instanceof ApiError) {
    const key = `changeErr_${err.code}` as keyof typeof loginI18n.th;
    if (key in loginI18n.th) {
      return key;
    }
    return 'errUnknown';
  }
  return 'errNetwork';
}

export function ChangePassword(props: {
  auth: Partial<PasswordApi>;
  onDone: () => void;
}): JSX.Element {
  const form = useForm();
  let formEl: HTMLFormElement | undefined;
  const [pending, setPending] = createSignal(false);
  const [errorKey, setErrorKey] = createSignal<keyof typeof loginI18n.th | undefined>(undefined);

  const onSubmit = form.submit(async () => {
    if (pending()) return;
    if (!formEl) return;

    const data = new FormData(formEl);
    const currentPassword = String(data.get('currentPassword') ?? '');
    const newPassword = String(data.get('newPassword') ?? '');
    const confirmPassword = String(data.get('confirmPassword') ?? '');

    if (newPassword !== confirmPassword) {
      setErrorKey('changeErrMismatch');
      return;
    }

    setPending(true);
    setErrorKey(undefined);

    try {
      if (!props.auth.changePassword) {
        throw new Error('changePassword unavailable');
      }
      await props.auth.changePassword({ currentPassword, newPassword });
      props.onDone();
    } catch (err) {
      setErrorKey(toChangeErrorKey(err));
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
    <section data-change-password="">
      <h2>{t('changeTitle')}</h2>
      <p>{t('changeIntro')}</p>
      <form ref={formEl} onSubmit={handleSubmit}>
        <PasswordField
          field={form.field('currentPassword')}
          label={t('currentPassword')}
          autocomplete="current-password"
          required
          maxLength={128}
        />
        <PasswordField
          field={form.field('newPassword')}
          label={t('newPassword')}
          autocomplete="new-password"
          required
          maxLength={128}
        />
        <PasswordField
          field={form.field('confirmPassword')}
          label={t('confirmPassword')}
          autocomplete="new-password"
          required
          maxLength={128}
        />
        <div>
          <button type="submit" disabled={pending()}>
            {t('changeSubmit')}
          </button>
        </div>
        <p data-change-message="" role="alert">
          {message()}
        </p>
      </form>
    </section>
  );
}
