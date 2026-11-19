import { createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { t } from '../i18n';
import { ThemeSwitch } from './ThemeSwitch';
import type { Key } from '../i18n/th';
import { sharedSession } from '../session/session';
import { AccountBar, roleLabel } from './AccountBar';
import type { QueueCardMode } from './QueueCard';
import { visibleStaffNav } from './staff-nav';
import { VersionFooter } from './VersionFooter';
import '@fontsource-variable/noto-sans-thai/wght.css';
import './xc-theme.css';
import './xc-theme-dark.css';
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
  staffNav?: boolean;
  queueCard?: QueueCardMode;
}): JSX.Element {
  const session = props.staffSession !== false ? sharedSession() : undefined;
  const nav = props.nav;
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
  const staffItems = () => visibleStaffNav(user()?.permissions ?? []);
  const path = () => (typeof window !== 'undefined' ? window.location.pathname : '');
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
      <header
        class="xc-shell-header"
        data-menu-open={menuOpen() ? 'true' : 'false'}
      >
        {session ? <span class="xc-shell-brand">XCHANGE × T.K.</span> : null}
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
            </div>
          </Show>
          {session && !nav && props.staffNav !== false ? (
            <Show when={staffItems().length > 0}>
              <div
                class="xc-shell-nav"
                onClick={(e) => {
                  if ((e.target as Element | null)?.closest?.('a')) setMenuOpen(false);
                }}
              >
                <nav>
                  <For each={staffItems()}>
                    {(item) => (
                      <a href={item.href} aria-current={path() === item.href ? 'page' : undefined}>
                        <span class="xc-shell-nav-icon" aria-hidden="true">
                          {item.icon}
                        </span>
                        {t(item.key as Key)}
                      </a>
                    )}
                  </For>
                </nav>
              </div>
            </Show>
          ) : null}
          {session && nav ? (
            <div
              class="xc-shell-nav"
              onClick={(e) => {
                if ((e.target as Element | null)?.closest?.('a')) setMenuOpen(false);
              }}
            >
              {nav}
            </div>
          ) : null}
          <Show when={user()}>
            <ThemeSwitch />
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
        {!session && nav ? <div class="xc-shell-nav">{nav}</div> : null}
        {!session ? <h1 class="xc-shell-title">{props.title}</h1> : null}
        <LangSwitch />
        {session ? <AccountBar session={session} queueCard={props.queueCard} /> : null}
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
          <div class="xc-content">
            {session ? <h1 class="xc-shell-title">{props.title}</h1> : null}
            {props.children}
          </div>
        ) : (
          <main class="xc-shell-main">
            {session ? <h1 class="xc-shell-title">{props.title}</h1> : null}
            {props.children}
          </main>
        )}
      </fieldset>
      <div class="xc-shell-footer">
        <VersionFooter />
      </div>
    </div>
  );
}
