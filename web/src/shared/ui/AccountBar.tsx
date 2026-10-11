import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { t } from '../i18n';
import { sharedSession, type SessionStore } from '../session/session';
import { QueueCard } from './QueueCard';

function cleanUrl(): string {
  if (typeof window === 'undefined') return '';
  const url = new URL(window.location.href);
  url.searchParams.delete('branch');
  const search = url.searchParams.toString();
  return `${url.pathname}${search ? `?${search}` : ''}${url.hash}`;
}

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
  const [open, setOpen] = createSignal(false);
  const [error, setError] = createSignal('');

  let menuRef: HTMLDivElement | undefined;
  let menuButtonRef: HTMLButtonElement | undefined;
  let reloaded = false;

  onMount(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && open()) {
        setOpen(false);
      }
    };
    const handleClickOutside = (e: MouseEvent) => {
      if (
        open() &&
        menuRef &&
        !menuRef.contains(e.target as Node) &&
        menuButtonRef &&
        !menuButtonRef.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    document.addEventListener('click', handleClickOutside);
    onCleanup(() => {
      document.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('click', handleClickOutside);
    });
  });

  createEffect(() => {
    if (session().ended?.() === 'branchChanged' && !reloaded) {
      reloaded = true;
      navigate(cleanUrl());
    }
  });

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

  const handleSwitch = async (branchId: string) => {
    setOpen(false);
    setError('');
    try {
      const sw = session().switchBranch;
      if (sw) {
        await sw(branchId);
      }
      navigate(cleanUrl());
    } catch {
      setError(t('shell.account.switchFailed'));
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

  const branchName = () => {
    const u = user() as ({ branch?: string; branchName?: string; branches?: unknown[] } | undefined);
    if (!u) return undefined;
    if (u.branchName !== undefined) {
      return u.branchName.trim() ? u.branchName : undefined;
    }
    if (u.branches === undefined && u.branch) {
      return u.branch;
    }
    return undefined;
  };
  const branches = () => (user() as any)?.branches as Array<{ id: string; code: string; name: string }> | undefined;

  return (
    <Show when={user()}>
      {(u) => (
        <div class="account-bar">
          <span>{userText()}</span>
          <Show when={error()}>
            <span>{error()}</span>
          </Show>
          <Show when={branchName()}>
            <Show
              when={branches() && branches()!.length > 1}
              fallback={<span>{t('shell.account.branch', { branch: branchName()! })}</span>}
            >
              <button
                ref={(el) => (menuButtonRef = el)}
                type="button"
                aria-haspopup="menu"
                aria-expanded={open() ? 'true' : 'false'}
                onClick={() => setOpen(!open())}
              >
                {t('shell.account.branch', { branch: branchName()! })}
              </button>
              <Show when={open()}>
                <div
                  ref={(el) => (menuRef = el)}
                  role="menu"
                  aria-label={t('shell.account.switchBranch')}
                >
                  <For each={branches()}>
                    {(b) => (
                      <div
                        role="menuitemradio"
                        aria-checked={b.id === u().branch ? 'true' : 'false'}
                        onClick={() => handleSwitch(b.id)}
                      >
                        {b.name}
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </Show>
          <button type="button" disabled={pending()} onClick={handleSignOut}>
            {t('shell.account.signOut')}
          </button>
          <QueueCard session={session()} />
        </div>
      )}
    </Show>
  );
}
