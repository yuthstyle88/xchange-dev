import { createSignal, type JSX } from 'solid-js';
import { ApiError } from '../../shared/api/client';
import { screenT } from '../../shared/i18n';
import { fieldError } from '../../shared/i18n/field-error';
import { PasswordField } from '../../shared/ui/PasswordField';
import { useForm } from '../../shared/ui/form';
import type { PasswordApi } from './auth-api';
import * as loginI18n from './i18n';

const t = screenT('login', loginI18n);

export function ChangePassword(props: {
  auth: Partial<PasswordApi>;
  onDone: () => void;
}): JSX.Element {
  const form = useForm();
  let formEl: HTMLFormElement | undefined;
  const [pending, setPending] = createSignal(false);
  const [errorMsg, setErrorMsg] = createSignal<string>('');
  const [newPasswordError, setNewPasswordError] = createSignal<string | undefined>(undefined);

  const onSubmit = form.submit(async () => {
    if (pending()) return;
    if (!formEl) return;

    const data = new FormData(formEl);
    const currentPassword = String(data.get('currentPassword') ?? '');
    const newPassword = String(data.get('newPassword') ?? '');
    const confirmPassword = String(data.get('confirmPassword') ?? '');

    setNewPasswordError(undefined);
    setErrorMsg('');

    if (newPassword.length < 12 || newPassword.length > 128) {
      setNewPasswordError(t('newPasswordHint'));
      return;
    }

    if (newPassword !== confirmPassword) {
      setErrorMsg(t('changeErrMismatch'));
      return;
    }

    setPending(true);

    try {
      if (!props.auth.changePassword) {
        throw new Error('changePassword unavailable');
      }
      await props.auth.changePassword({ currentPassword, newPassword });
      props.onDone();
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.code === 'validationFailed' || err.field === 'newPassword') {
          const fe = fieldError(err, { newPassword: t('newPassword') });
          if (fe.field === 'newPassword') setNewPasswordError(fe.message);
          setErrorMsg(fe.message);
        } else {
          const key = `changeErr_${err.code}` as keyof typeof loginI18n.th;
          setErrorMsg(key in loginI18n.th ? t(key) : t('errUnknown'));
        }
      } else {
        setErrorMsg(t('errNetwork'));
      }
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
          minLength={12}
          maxLength={128}
          hint={t('newPasswordHint')}
          error={newPasswordError()}
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
          {errorMsg()}
        </p>
      </form>
    </section>
  );
}
