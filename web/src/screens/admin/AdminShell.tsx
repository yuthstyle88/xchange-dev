// V2-shell (#460): admin shell layout, routing, branch picker and menu host (§7, §9, §10; design §12.3–§12.6, §12.14)

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { Dynamic } from 'solid-js/web';
import type { ApiClient } from '../../shared/api/client';
import type { AdminBranchesData, AdminBranchItem } from '../../shared/api/generated/admin_branches';
import type { Me } from '../../shared/api/generated/common';
import { sharedSession } from '../../shared/session/session';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { AppShell } from '../../shared/ui/AppShell';
import { Dialog, openDialog } from './parts/Dialog';
import { Toast } from './parts/toast';
import { pendingWrite } from './parts/pending-write';
import { requestRefresh } from './parts/refresh';
import { en, th } from './i18n';
import type { AdminMenu } from './menus';
import { QueueBar } from './queue/QueueBar';
import './admin.css';

export interface AdminShellProps {
  api: ApiClient;
  menus: AdminMenu[];
}

function getBangkokToday(): string {
  const now = new Date();
  const formatter = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Bangkok',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
  return formatter.format(now);
}

function formatTime(d: Date): string {
  return new Intl.DateTimeFormat('en-GB', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
    timeZone: 'Asia/Bangkok',
  }).format(d);
}

export function AdminShell(props: AdminShellProps): JSX.Element {
  const t = screenT('admin', { th, en });

  const [currentBranch, setSessionBranch] = createSignal('');
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [branchError, setBranchError] = createSignal<unknown>(null);
  const [isOnline, setIsOnline] = createSignal(true);
  const [lastSync, setLastSync] = createSignal(new Date());
  const [permissions, setPermissions] = createSignal<readonly string[]>([]);
  const [ready, setReady] = createSignal(false);

  // Hash routing
  const [currentHash, setCurrentHash] = createSignal(
    typeof window !== 'undefined' ? window.location.hash : '',
  );

  // Search
  const [searchQuery, setSearchQuery] = createSignal('');

  // Mobile / Narrow detection
  const [isNarrow, setIsNarrow] = createSignal(
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia('(max-width: 900px)').matches
      : false,
  );

  // Mobile menu
  const [mobileOpen, setMobileOpen] = createSignal(false);
  let menuButtonRef: HTMLButtonElement | undefined;
  let sidebarRef: HTMLElement | undefined;
  let searchInputRef: HTMLInputElement | undefined;

  const setMenu = (open: boolean) => {
    if (mobileOpen() === open) return;
    setMobileOpen(open);
    if (!open) {
      menuButtonRef?.focus();
    } else {
      if (isNarrow()) {
        searchInputRef?.focus();
      }
    }
  };

  createEffect(() => {
    if (sidebarRef) {
      const shouldBeInert = !mobileOpen() && isNarrow();
      if (shouldBeInert) {
        sidebarRef.setAttribute('inert', '');
        sidebarRef.inert = true;
      } else {
        sidebarRef.removeAttribute('inert');
        sidebarRef.inert = false;
      }
    }
  });

  // Period state
  const initialToday = getBangkokToday();
  const [periodFrom, setPeriodFrom] = createSignal(initialToday);
  const [periodTo, setPeriodTo] = createSignal(initialToday);
  const [validPeriod, setValidPeriod] = createSignal({
    from: initialToday,
    to: initialToday,
  });

  const isPeriodValid = createMemo(() => {
    const f = periodFrom();
    const toVal = periodTo();
    if (f !== '' && toVal !== '' && f > toVal) {
      return false;
    }
    return true;
  });

  createEffect(() => {
    if (isPeriodValid()) {
      setValidPeriod({ from: periodFrom(), to: periodTo() });
    }
  });

  const applyHashQuery = (hashString: string) => {
    const raw = hashString.startsWith('#') ? hashString.slice(1) : hashString;
    const parts = raw.split('?');
    if (parts.length > 1) {
      const q = new URLSearchParams(parts[1]);
      if (q.has('from')) {
        setPeriodFrom(q.get('from') || '');
      }
      if (q.has('to')) {
        setPeriodTo(q.get('to') || '');
      }
    }
  };

  const onHashChange = () => {
    const h = window.location.hash;
    setCurrentHash(h);
    applyHashQuery(h);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      if (mobileOpen()) {
        setMenu(false);
      }
    } else if (e.key === 'Tab' && mobileOpen() && isNarrow() && sidebarRef) {
      if (document.activeElement?.closest('dialog')) return;
      const focusables = Array.from(
        sidebarRef.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );
      if (focusables.length > 0) {
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (e.shiftKey) {
          if (
            document.activeElement === first ||
            !sidebarRef.contains(document.activeElement)
          ) {
            e.preventDefault();
            last.focus();
          }
        } else {
          if (
            document.activeElement === last ||
            !sidebarRef.contains(document.activeElement)
          ) {
            e.preventDefault();
            first.focus();
          }
        }
      }
    }
  };

  onMount(() => {
    window.addEventListener('hashchange', onHashChange);
    document.addEventListener('keydown', onKeyDown);
    applyHashQuery(window.location.hash);

    if (
      typeof window !== 'undefined' &&
      typeof window.matchMedia === 'function'
    ) {
      const mql = window.matchMedia('(max-width: 900px)');
      const handler = (e: MediaQueryListEvent | { matches: boolean }) => {
        setIsNarrow(e.matches);
        if (!e.matches) {
          setMenu(false);
        }
      };
      mql.addEventListener('change', handler);
      onCleanup(() => mql.removeEventListener('change', handler));
    }
  });

  onCleanup(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener('hashchange', onHashChange);
      document.removeEventListener('keydown', onKeyDown);
    }
  });

  const fetchBranches = async (activeBranch: string) => {
    try {
      const res = await props.api.view<AdminBranchesData>(
        'admin_branches',
        activeBranch ? { branch: activeBranch } : {},
      );
      if (res.data) {
        setBranches(res.data.branches);
        setBranchError(null);
        setIsOnline(true);
        setLastSync(new Date());
        return res.data.branches;
      }
    } catch (err: unknown) {
      setIsOnline(false);
      setBranchError(err);
    }
    return [];
  };

  let meAnswered = false;

  const loadMeAndBranches = async () => {
    let branchFromMe = '';
    try {
      const meRes = await props.api.view<Me>('me');
      if (meRes.data) {
        meAnswered = true;
        branchFromMe = meRes.data.branch || '';
        setPermissions(meRes.data.permissions);
        setBranchError(null);
        setIsOnline(true);
        setLastSync(new Date());
        sharedSession().updateMe?.(meRes.data);
      }
    } catch (err: unknown) {
      setIsOnline(false);
      setBranchError(err);
    }

    setSessionBranch(branchFromMe);

    if (branchFromMe) {
      await fetchBranches(branchFromMe);
    }
  };

  onMount(async () => {
    await loadMeAndBranches();
    setReady(true);
  });

  // Menus sorted by order
  const sortedMenus = createMemo(() => {
    return [...props.menus].sort((a, b) => a.order - b.order);
  });

  // Allowed menus: `needs` is 'all' or a permission `me.permissions` holds (#1030 §3.10)
  const allowedMenus = createMemo(() => {
    const held = permissions();
    return sortedMenus().filter((m) => m.needs === 'all' || held.includes(m.needs));
  });

  // Active menu calculation
  const activeMenu = createMemo(() => {
    const list = allowedMenus();
    if (list.length === 0) return null;

    const raw = currentHash().startsWith('#')
      ? currentHash().slice(1)
      : currentHash();
    const id = raw.split('?')[0];

    if (id) {
      const found = list.find((m) => m.id === id);
      if (found) return found;
    }

    // Default to lowest order menu
    return list[0];
  });

  // Set document.title from shown menu heading (Controller Decision 6, admin-html:158)
  createEffect(() => {
    const menu = activeMenu();
    if (menu) {
      document.title = `XCHANGE · ${menu.heading.title()}`;
    } else {
      document.title = t('pageTitle');
    }
  });

  const currentBranchName = createMemo(() => {
    const id = currentBranch();
    const found = branches().find((b) => b.id === id);
    return found ? found.name : id;
  });

  const handleRefresh = async () => {
    requestRefresh();
    if (!meAnswered) {
      await loadMeAndBranches();
      return;
    }
    await fetchBranches(currentBranch());
  };

  // Add branch dialog
  const openAddBranchDialog = () => {
    let nameInput = '';
    const [dialogError, setDialogError] = createSignal('');

    openDialog(t('addTitle'), () => {
      const handleAddSubmit = (e: Event) => {
        e.preventDefault();
        const trimmed = nameInput.trim();
        if (
          !trimmed ||
          branches().some(
            (b) => b.name.toLowerCase() === trimmed.toLowerCase(),
          )
        ) {
          setDialogError(t('addError'));
          return;
        }

        void pendingWrite.run(
          props.api,
          currentBranch(),
          'adminCreateBranch',
          { name: trimmed },
          async () => {
            await fetchBranches(currentBranch());
          },
        );
      };

      return (
        <div class="add-branch-dialog">
          <p class="add-note">{t('addNote')}</p>
          <Show when={dialogError()}>
            <p class="add-error" role="alert">
              {dialogError()}
            </p>
          </Show>
          <form onSubmit={handleAddSubmit}>
            <input
              type="text"
              placeholder={t('addPlaceholder')}
              maxLength={60}
              onInput={(e) => {
                nameInput = e.currentTarget.value;
                setDialogError('');
              }}
            />
            <button type="submit" class="primary">{t('addSubmit')}</button>
          </form>
        </div>
      );
    });
  };

  const openConnectionDialog = () => {
    openDialog(t('connectionTitle'), () => (
      <div class="connection-dialog">
        <p>{t('connection1')}</p>
        <p>{t('connection2')}</p>
      </div>
    ));
  };

  // Filtered links and groups by search
  const isQueryMatch = (label: string) => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return true;
    return label.toLowerCase().includes(q);
  };

  const hasAnyMatch = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return true;

    for (const m of allowedMenus()) {
      if (isQueryMatch(m.label())) return true;
    }
    if (isQueryMatch(t('linkQueue'))) return true;
    if (isQueryMatch(t('linkKiosk'))) return true;
    if (isQueryMatch(t('linkDevices'))) return true;
    if (isQueryMatch(t('linkGuide'))) return true;

    return false;
  });

  const deskMatches = createMemo(() => {
    return (
      allowedMenus().some((m) => m.group === 'desk' && isQueryMatch(m.label())) ||
      isQueryMatch(t('linkQueue'))
    );
  });

  const moneyMatches = createMemo(() => {
    return allowedMenus().some(
      (m) => m.group === 'money' && isQueryMatch(m.label()),
    );
  });

  const dataMatches = createMemo(() => {
    return allowedMenus().some(
      (m) => m.group === 'data' && isQueryMatch(m.label()),
    );
  });

  const toolsMatches = createMemo(() => {
    return (
      allowedMenus().some((m) => m.group === 'tools' && isQueryMatch(m.label())) ||
      isQueryMatch(t('linkKiosk')) ||
      isQueryMatch(t('linkDevices'))
    );
  });

  return (
    <AppShell
      look="admin"
      title={t('pageTitle')}
      ownsMain={true}
      nav={
        <button
          ref={(el) => (menuButtonRef = el)}
          type="button"
          aria-controls="admin-sidebar"
          aria-expanded={mobileOpen() ? 'true' : 'false'}
          onClick={() => setMenu(!mobileOpen())}
        >
          {t('menuButton')}
        </button>
      }
    >
      <div
        class="admin-shell-container"
        classList={{ 'workspace-menu-open': mobileOpen() }}
      >
        {/* Sidebar */}
        <aside
          ref={(el) => (sidebarRef = el)}
          id="admin-sidebar"
          class={mobileOpen() ? 'admin-sidebar open' : 'admin-sidebar'}
        >
          {/* Branch management header */}
          <div class="branch-manager">
            <div>{t('manageBranches')}</div>
            <button
              type="button"
              disabled={!isOnline()}
              onClick={openAddBranchDialog}
            >
              {t('addBranch')}
            </button>
            <button type="button" onClick={handleRefresh}>
              {t('refresh')}
            </button>
          </div>

          {/* Search box */}
          <div class="menu-search">
            <input
              ref={(el) => (searchInputRef = el)}
              type="text"
              placeholder={t('searchPlaceholder')}
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
            />
          </div>

          <Show when={!hasAnyMatch()}>
            <div class="no-menu-notice">{t('noMenu')}</div>
          </Show>

          {/* Nav groups */}
          <nav
            class="admin-nav"
            onClick={(e) => {
              const target = e.target as HTMLElement | null;
              if (target?.closest('a')) {
                setMenu(false);
              }
            }}
          >
            {/* Group 1: Desk */}
            <Show when={deskMatches()}>
              <div class="nav-group">
                <h3>{t('groupDesk')}</h3>
                <For each={allowedMenus().filter((m) => m.group === 'desk')}>
                  {(menu) => (
                    <Show when={isQueryMatch(menu.label())}>
                      <a
                        href={`#${menu.id}`}
                        aria-current={
                          activeMenu()?.id === menu.id ? 'page' : undefined
                        }
                      >
                        <span aria-hidden="true">{menu.icon}</span>{' '}
                        <span>{menu.label()}</span>
                      </a>
                    </Show>
                  )}
                </For>
                <Show when={isQueryMatch(t('linkQueue'))}>
                  <a href="/kiosk-queue.html">
                    <span aria-hidden="true">≡</span>{' '}
                    <span>{t('linkQueue')}</span>
                  </a>
                </Show>
              </div>
            </Show>

            {/* Group 2: Money */}
            <Show when={moneyMatches()}>
              <div class="nav-group">
                <h3>{t('groupMoney')}</h3>
                <For each={allowedMenus().filter((m) => m.group === 'money')}>
                  {(menu) => (
                    <Show when={isQueryMatch(menu.label())}>
                      <a
                        href={`#${menu.id}`}
                        aria-current={
                          activeMenu()?.id === menu.id ? 'page' : undefined
                        }
                      >
                        <span aria-hidden="true">{menu.icon}</span>{' '}
                        <span>{menu.label()}</span>
                      </a>
                    </Show>
                  )}
                </For>
              </div>
            </Show>

            {/* Group 3: Data */}
            <Show when={dataMatches()}>
              <div class="nav-group">
                <h3>{t('groupData')}</h3>
                <For each={allowedMenus().filter((m) => m.group === 'data')}>
                  {(menu) => (
                    <Show when={isQueryMatch(menu.label())}>
                      <a
                        href={`#${menu.id}`}
                        aria-current={
                          activeMenu()?.id === menu.id ? 'page' : undefined
                        }
                      >
                        <span aria-hidden="true">{menu.icon}</span>{' '}
                        <span>{menu.label()}</span>
                      </a>
                    </Show>
                  )}
                </For>
              </div>
            </Show>

            {/* Group 4: Tools */}
            <Show when={toolsMatches()}>
              <div class="nav-group">
                <h3>{t('groupTools')}</h3>
                <For each={allowedMenus().filter((m) => m.group === 'tools')}>
                  {(menu) => (
                    <Show when={isQueryMatch(menu.label())}>
                      <a
                        href={`#${menu.id}`}
                        aria-current={
                          activeMenu()?.id === menu.id ? 'page' : undefined
                        }
                      >
                        <span aria-hidden="true">{menu.icon}</span>{' '}
                        <span>{menu.label()}</span>
                      </a>
                    </Show>
                  )}
                </For>
                <Show when={isQueryMatch(t('linkKiosk'))}>
                  <a
                    href="/kiosk"
                    target="_blank"
                    rel="noopener"
                  >
                    <span aria-hidden="true">↗</span>{' '}
                    <span>{t('linkKiosk')}</span>
                  </a>
                </Show>
                <Show when={isQueryMatch(t('linkDevices'))}>
                  <a href="/devices">
                    <span aria-hidden="true">⎙</span>{' '}
                    <span>{t('linkDevices')}</span>
                  </a>
                </Show>
              </div>
            </Show>

            {/* Standalone Guide link */}
            <Show when={isQueryMatch(t('linkGuide'))}>
              <div class="nav-guide">
                <a href="/staff-guide.html">
                  <span aria-hidden="true">?</span>{' '}
                  <span>{t('linkGuide')}</span>
                </a>
              </div>
            </Show>
          </nav>
        </aside>

        <Show when={mobileOpen() && isNarrow()}>
          <button
            type="button"
            class="workspace-shade"
            aria-label={t('closeMenu')}
            onClick={() => setMenu(false)}
          />
        </Show>

        {/* Main Content Area */}
        <main class="admin-main">
          {/* Top Notice Bar */}
          <div role="status" class="notice-bar">
            <Show
              when={isOnline()}
              fallback={
                <span>
                  {t('liveOffline')} · {branchError() != null ? wordedError(branchError()) : ''}
                </span>
              }
            >
              <span>
                {t('liveOnline')} · {t('lastSync')}
                {formatTime(lastSync())}
              </span>
            </Show>
          </div>

          {/* Connection info button */}
          <div class="connection-action">
            <button
              type="button"
              class="desk-link"
              onClick={openConnectionDialog}
            >
              {t('connection')}
            </button>
          </div>

          {/* Menu Heading */}
          <Show when={activeMenu()}>
            {(menu) => (
              <header class="menu-heading">
                <div class="eyebrow">{menu().heading.eyebrow()}</div>
                <h2 class="title">{menu().heading.title()}</h2>
                <p class="subtitle">
                  {menu().heading.subtitle()} · {currentBranchName()}
                </p>
              </header>
            )}
          </Show>

          {/* Period Toolbar (dated menus only) */}
          <Show when={activeMenu()?.dated}>
            <div class="period-toolbar">
              <input
                type="date"
                value={periodFrom()}
                onInput={(e) => setPeriodFrom(e.currentTarget.value)}
              />
              <input
                type="date"
                value={periodTo()}
                onInput={(e) => setPeriodTo(e.currentTarget.value)}
              />
              <button
                type="button"
                onClick={() => {
                  const todayStr = getBangkokToday();
                  setPeriodFrom(todayStr);
                  setPeriodTo(todayStr);
                }}
              >
                {t('today')}
              </button>
              <button
                type="button"
                onClick={() => {
                  setPeriodFrom('');
                  setPeriodTo('');
                }}
              >
                {t('allDates')}
              </button>
              <Show when={!isPeriodValid()}>
                <p class="date-error" role="alert">
                  {t('dateError')}
                </p>
              </Show>
            </div>
          </Show>

          {/* Active Menu Content Host */}
          <div class="menu-content">
            <Show when={ready() && activeMenu()}>
              {(menu) => (
                <Dynamic
                  component={menu().Component}
                  branch={currentBranch}
                  period={validPeriod}
                  shown={() => true}
                />
              )}
            </Show>
          </div>

          {/* Footer Receipt Sample link */}
          <footer class="admin-footer">
            <a href="/receipt-sample" target="_blank" rel="noopener">
              {t('linkReceiptSample')}
            </a>
          </footer>
        </main>
      </div>

      <Show when={permissions().includes('QueueOperate')}>
        <QueueBar branch={currentBranch} branchLabel={currentBranchName} />
      </Show>
      <Dialog />
      <Toast />
    </AppShell>
  );
}
