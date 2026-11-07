import { createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { t } from '../i18n';
import type { Key } from '../i18n/th';
import { sharedSession } from '../session/session';
import { AccountBar, roleLabel } from './AccountBar';
import { VersionFooter } from './VersionFooter';
import './xc-theme.css';
import './shell.css';

export type AppShellLook = 'devices' | 'kiosk' | 'admin';

export function AppShell(props: {
  title: string;
  nav?: JSX.Element;
  children?: JSX.Element;
  ownsMain?: boolean;
  look?: AppShellLook;
  banner?: JSX.Element;
  staffSession?: boolean;
}): JSX.Element {
  const session = props.staffSession !== false ? sharedSession() : undefined;
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [pending, setPending] = createSignal(false);
  const [signOutError, setSignOutError] = createSignal('');
  let menuBtnRef: HTMLButtonElement | undefined;

  onMount(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && menuOpen()) {
        setMenuOpen(false);
        menuBtnRef?.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    onCleanup(() => document.removeEventListener('keydown', onKeyDown));
  });

  const user = () => session?.me();
  const userText = () => {
    const u = user();
    if (!u) return '';
    const roles = u.roles
      .filter((r) => r.branchId === null || r.branchId === u.branch)
      .map((r) => roleLabel(r.role))
      .join(', ');
    if (u.name == null) return roles;
    return roles ? `${u.name} · ${roles}` : u.name;
  };
  const branchName = () => {
    const u = user() as ({ branch?: string; branchName?: string; branches?: unknown[] } | undefined);
    if (!u) return undefined;
    if (u.branchName !== undefined) return u.branchName.trim() ? u.branchName : undefined;
    if (u.branches === undefined && u.branch) return u.branch;
    return undefined;
  };

  const handleSignOut = async () => {
    if (!session || pending()) return;
    setPending(true);
    setSignOutError('');
    try {
      await session.signOut();
      setMenuOpen(false);
      if (typeof window !== 'undefined') {
        window.location.assign('/login');
      }
    } catch {
      setSignOutError(t('shell.account.signOutFailed'));
      setPending(false);
    }
  };

  return (
    <div class={props.look ? `xc-shell xc-theme xc-${props.look}` : 'xc-shell xc-theme'}>
      {props.banner ? <div class="xc-shell-banner">{props.banner}</div> : null}
      {session ? <AccountBar session={session} /> : null}
      <header
        class="xc-shell-header"
        data-menu-open={menuOpen() ? 'true' : 'false'}
      >
        {session ? (
          <button
            ref={(el) => (menuBtnRef = el)}
            type="button"
            class="xc-shell-menu-btn"
            aria-controls="xc-shell-menu"
            aria-expanded={menuOpen() ? 'true' : 'false'}
            aria-label={t('shell.menu')}
            onClick={() => setMenuOpen(!menuOpen())}
          >
            <span aria-hidden="true">{menuOpen() ? '✕' : '☰'}</span>
          </button>
        ) : null}
        <div
          id="xc-shell-menu"
          class="xc-shell-panel"
          data-open={menuOpen() ? 'true' : 'false'}
        >
          <Show when={user()}>
            <div class="xc-shell-panel-account">
              <span class="xc-shell-panel-user">{userText()}</span>
              <Show when={branchName()}>
                <span class="xc-shell-panel-branch">{t('shell.account.branch', { branch: branchName()! })}</span>
              </Show>
            </div>
          </Show>
          {session && props.nav ? (
            <div
              class="xc-shell-nav"
              onClick={(e) => {
                if ((e.target as Element | null)?.closest?.('a')) setMenuOpen(false);
              }}
            >
              {props.nav}
            </div>
          ) : null}
          <Show when={user()}>
            <div class="xc-shell-panel-footer">
              <button type="button" class="xc-shell-panel-signout" disabled={pending()} onClick={handleSignOut}>
                {t('shell.account.signOut')}
              </button>
              <Show when={signOutError()}>
                <p role="alert">{signOutError()}</p>
              </Show>
            </div>
          </Show>
        </div>
        {/* No staff session (the customer shell) has no menu button, so its nav stays in the header. */}
        {!session && props.nav ? <div class="xc-shell-nav">{props.nav}</div> : null}
        <h1 class="xc-shell-title">{props.title}</h1>
        <LangSwitch />
      </header>
      <Show when={session && session.ended()}>
        <p role="alert" class="xc-shell-alert">{t(`shell.session.${session!.ended()!}` as Key)}</p>
      </Show>
      <fieldset
        class="xc-lock"
        disabled={session ? session.ended() !== undefined : false}
        style="border: 0; padding: 0; margin: 0; min-width: 0"
      >
        {props.ownsMain ? (
          <div class="xc-content">{props.children}</div>
        ) : (
          <main class="xc-shell-main">{props.children}</main>
        )}
      </fieldset>
      <div class="xc-shell-footer">
        <VersionFooter />
      </div>
    </div>
  );
}
