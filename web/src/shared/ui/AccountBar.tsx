import { createSignal, Show, type JSX } from 'solid-js';
import { t } from '../i18n';
import { sharedSession, type SessionStore } from '../session/session';

export function AccountBar(props: {
  session?: SessionStore;
  navigate?: (url: string) => void;
}): JSX.Element {
  const session = () => props.session ?? sharedSession();
  const navigate = (url: string) => {
    if (props.navigate) {
      props.navigate(url);
    } else if (typeof window !== 'undefined') {
      window.location.assign(url);
    }
  };

  const [pending, setPending] = createSignal(false);

  const handleSignOut = async () => {
    if (pending()) return;
    setPending(true);
    try {
      await session().signOut();
      navigate('/login');
    } catch {
      setPending(false);
    }
  };

  const user = () => session().me();

  const userText = () => {
    const u = user();
    if (!u) return '';
    if (u.name != null && u.username != null) {
      return `${u.name} · ${u.username} · ${u.role}`;
    }
    return u.role;
  };

  return (
    <Show when={user()}>
      {(u) => (
        <div class="account-bar">
          <span>{userText()}</span>
          <span>{t('shell.account.branch', { branch: u().branch })}</span>
          <button type="button" disabled={pending()} onClick={handleSignOut}>
            {t('shell.account.signOut')}
          </button>
        </div>
      )}
    </Show>
  );
}
