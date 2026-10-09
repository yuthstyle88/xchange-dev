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
import { ApiError, type ApiClient } from '../../shared/api/client';
import type { AdminBranchesData, AdminBranchItem } from '../../shared/api/generated/admin_branches';
import type { Me } from '../../shared/api/generated/common';
import { errorText, screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { Dialog, openDialog } from './parts/Dialog';
import { Toast } from './parts/toast';
import { pendingWrite } from './parts/pending-write';
import { requestRefresh } from './parts/refresh';
import { en, th } from './i18n';
import type { AdminMenu } from './menus';
import { QueueBar } from './queue/QueueBar';

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

  let selectRef: HTMLSelectElement | undefined;

  const [currentBranch, setCurrentBranch] = createSignal('');
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [branchError, setBranchError] = createSignal<{
    code: string;
    message: string;
  } | null>(null);
  const [isOnline, setIsOnline] = createSignal(true);
  const [lastSync, setLastSync] = createSignal(new Date());
  const [userRole, setUserRole] = createSignal<string | undefined>(undefined);
  const [ready, setReady] = createSignal(false);

  createEffect(() => {
    const b = currentBranch();
    branches();
    if (selectRef && b) {
      selectRef.value = b;
    }
  });

  // Hash routing
  const [currentHash, setCurrentHash] = createSignal(
    typeof window !== 'undefined' ? window.location.hash : '',
  );

  // Search
  const [searchQuery, setSearchQuery] = createSignal('');

  // Mobile menu
  const [mobileOpen, setMobileOpen] = createSignal(false);

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
      setMobileOpen(false);
    }
  };

  onMount(() => {
    window.addEventListener('hashchange', onHashChange);
    document.addEventListener('keydown', onKeyDown);
    applyHashQuery(window.location.hash);
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
      if (err instanceof ApiError) {
        setBranchError({ code: err.code, message: err.message });
      } else if (err instanceof Error) {
        setBranchError({ code: '', message: err.message });
      } else {
        setBranchError({ code: '', message: String(err) });
      }
    }
    return [];
  };

  onMount(async () => {
    const searchBranch =
      typeof window !== 'undefined'
        ? new URLSearchParams(window.location.search).get('branch')
        : null;

    let branchFromMe = '';
    try {
      const meRes = await props.api.view<Me>(
        'me',
        searchBranch ? { branch: searchBranch } : {},
      );
      if (meRes.data) {
        branchFromMe = meRes.data.branch || '';
        setUserRole(meRes.data.role);
      }
    } catch {
      // ignore
    }

    const initialPick = branchFromMe || searchBranch || 'MAIN';
    setCurrentBranch(initialPick);

    const loadedBranches = await fetchBranches(initialPick);
    if (loadedBranches.length > 0) {
      if (!loadedBranches.some((b) => b.id === initialPick)) {
        setCurrentBranch(loadedBranches[0].id);
      }
    }

    setReady(true);
  });

  // Menus sorted by order
  const sortedMenus = createMemo(() => {
    return [...props.menus].sort((a, b) => a.order - b.order);
  });

  // Allowed menus for current role
  const allowedMenus = createMemo(() => {
    const role = userRole();
    return sortedMenus().filter((m) => {
      if (m.roles === 'all') return true;
      if (!role) return false;
      return m.roles.includes(role);
    });
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

  const handleBranchChange = (
    e: Event & { currentTarget: HTMLSelectElement },
  ) => {
    if (pendingWrite.busy()) {
      e.preventDefault();
      e.currentTarget.value = currentBranch();
      return;
    }
    const next = e.currentTarget.value;
    setCurrentBranch(next);
  };

  const handleRefresh = async () => {
    requestRefresh();
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
            const updated = await fetchBranches(currentBranch());
            // Controller Decision 3: switch to new branch
            const created = updated.find(
              (b) => b.name.toLowerCase() === trimmed.toLowerCase(),
            );
            if (created) {
              setCurrentBranch(created.id);
            }
          },
        );
      };

      return (
        <div class="add-branch-dialog">
          <p class="add-note">{t('addNote')}</p>
          <Show when={dialogError()}>
            <p class="add-error" role="alert" style={{ color: 'red' }}>
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
            <button type="submit">{t('addSubmit')}</button>
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
      title={t('pageTitle')}
      ownsMain={true}
      nav={
        <button
          type="button"
          aria-controls="admin-sidebar"
          aria-expanded={mobileOpen() ? 'true' : 'false'}
          onClick={() => setMobileOpen(!mobileOpen())}
        >
          {t('menuButton')}
        </button>
      }
    >
      <div class="admin-shell-container">
        {/* Sidebar */}
        <aside
          id="admin-sidebar"
          class="admin-sidebar"
          style={mobileOpen() ? {} : undefined}
        >
          {/* Branch management header */}
          <div class="branch-manager">
            <div>{t('manageBranches')}</div>
            <select
              ref={selectRef}
              disabled={pendingWrite.busy()}
              value={currentBranch()}
              onChange={handleBranchChange}
            >
              <For each={branches()}>
                {(b) => (
                  <option value={b.id} selected={b.id === currentBranch()}>
                    {b.name}
                  </option>
                )}
              </For>
            </select>
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
          <nav class="admin-nav">
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
                  <a href={`/kiosk-queue.html?branch=${currentBranch()}`}>
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
                    href={`/kiosk?branch=${currentBranch()}`}
                    target="_blank"
                    rel="noopener"
                  >
                    <span aria-hidden="true">↗</span>{' '}
                    <span>{t('linkKiosk')}</span>
                  </a>
                </Show>
                <Show when={isQueryMatch(t('linkDevices'))}>
                  <a href={`/devices?branch=${currentBranch()}`}>
                    <span aria-hidden="true">⎙</span>{' '}
                    <span>{t('linkDevices')}</span>
                  </a>
                </Show>
              </div>
            </Show>

            {/* Standalone Guide link */}
            <Show when={isQueryMatch(t('linkGuide'))}>
              <div class="nav-guide">
                <a href={`/staff-guide.html?branch=${currentBranch()}`}>
                  <span aria-hidden="true">?</span>{' '}
                  <span>{t('linkGuide')}</span>
                </a>
              </div>
            </Show>
          </nav>
        </aside>

        {/* Main Content Area */}
        <main class="admin-main">
          {/* Top Notice Bar */}
          <div role="status" class="notice-bar">
            <Show
              when={isOnline()}
              fallback={
                <span>
                  {t('liveOffline')} ·{' '}
                  {branchError()
                    ? errorText(branchError()!.code, branchError()!.message)
                    : ''}
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
            <button type="button" onClick={openConnectionDialog}>
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
                <p class="date-error" role="alert" style={{ color: 'red' }}>
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

      <QueueBar branch={currentBranch} branchLabel={currentBranchName} />
      <Dialog />
      <Toast />
    </AppShell>
  );
}
