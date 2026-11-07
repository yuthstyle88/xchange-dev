// APP-7 (#998) the customer app shell (design customer-app.md §4.5; contract issuecomment-6137548682; LK-1 ruling
// issuecomment-6137816294). It composes AppShell, as AdminShell does, and adds the customer's bottom navigation and
// sign-in state. AppShell's header carries LangSwitch, which shows TH / EN / 中文 for the customer audience.
// A screen never reads my_profile or the language itself.
import { createResource, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { MyProfileData } from '../../../shared/api/generated/my_profile';
import { screenT } from '../../../shared/i18n';
import { readTabletMode } from '../../../shared/session/tablet';
import { AppShell } from '../../../shared/ui/AppShell';
import * as table from './i18n';
import logoUrl from './xc-logo.png';
import './xc-customer-tokens.css';
import './shell.css';

const tr = screenT('customer._shell', table);

export type CustomerRoute =
  | '/app'
  | '/app/history'
  | '/app/branches'
  | '/app/journey'
  | '/app/book'
  | '/app/bookings'
  | '/app/queue'
  | '/app/assistant'
  | '/app/rate-alerts'
  | '/app/notifications'
  | '/app/member'
  | '/app/profile'
  | '/app/sign-in'
  | '/app/sign-up'
  | '/app/verification';

type NavKey = 'home' | 'branches' | 'bookings' | 'history' | 'assistant';

/** The bottom navigation's icons (24 px line icons, drawn in the link's colour); one fresh node per render. */
const ICON: Record<NavKey, () => JSX.Element> = {
  home: () => <path d="M3 11.5 12 4l9 7.5M5.5 9.5V20h13V9.5M10 20v-5.5h4V20" />,
  branches: () => (
    <>
      <path d="M12 21s-6.5-6.1-6.5-11a6.5 6.5 0 0 1 13 0c0 4.9-6.5 11-6.5 11Z" />
      <circle cx="12" cy="10" r="2.4" />
    </>
  ),
  bookings: () => (
    <>
      <rect x="4" y="5.5" width="16" height="14.5" rx="2.5" />
      <path d="M4 10h16M8.5 3.5v4M15.5 3.5v4M9 14.5l2 2 4-4" />
    </>
  ),
  history: () => (
    <>
      <path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 9" />
      <path d="M4.5 4.5V9H9M12 8v4.5l3 2" />
    </>
  ),
  assistant: () => <path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 17h-7l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5ZM8 11h.01M12 11h.01M16 11h.01" />,
};

/** The bottom navigation, in order. */
const NAV: ReadonlyArray<{ href: CustomerRoute; key: NavKey }> = [
  { href: '/app', key: 'home' },
  { href: '/app/branches', key: 'branches' },
  { href: '/app/bookings', key: 'bookings' },
  { href: '/app/history', key: 'history' },
  { href: '/app/assistant', key: 'assistant' },
];

/** The navigation item a route belongs to: a route with no item of its own marks its parent, and the notifications
 * page (reached from the bar) marks none. */
const NAV_OF: Record<CustomerRoute, CustomerRoute | undefined> = {
  '/app': '/app',
  '/app/journey': '/app',
  '/app/branches': '/app/branches',
  '/app/book': '/app/bookings',
  '/app/bookings': '/app/bookings',
  '/app/queue': '/app/bookings',
  '/app/history': '/app/history',
  '/app/assistant': '/app/assistant',
  '/app/rate-alerts': undefined,
  '/app/notifications': undefined,
  '/app/member': undefined,
  '/app/profile': undefined,
  '/app/sign-in': undefined,
  '/app/sign-up': undefined,
  '/app/verification': undefined,
};

type Profile = { signedIn: true; name: string } | { signedIn: false };

/** The signed-in customer from `my_profile`; a `401` is "signed out", any other failure shows neither. */
async function loadProfile(): Promise<Profile | undefined> {
  try {
    const answer = await new ApiClient().view<MyProfileData>('my_profile');
    return answer.data ? { signedIn: true, name: answer.data.displayName } : undefined;
  } catch (err: unknown) {
    return (err as { status?: number }).status === 401 ? { signedIn: false } : undefined;
  }
}

export function CustomerShell(props: {
  route: CustomerRoute;
  tabletFlow?: boolean;
  children?: JSX.Element;
}): JSX.Element {
  const [profile] = createResource(loadProfile);
  const [paired, setPaired] = createSignal(false);
  if (props.tabletFlow) {
    void readTabletMode(fetch).then((mode) => {
      if (mode.paired) setPaired(true);
    });
  }
  const current = () => NAV_OF[props.route];

  return (
    <div class="xc-customer">
      <AppShell
        title={tr('title')}
        ownsMain
        staffSession={false}
        nav={
          <span class="xc-customer-logo">
            <img src={logoUrl} alt="XCHANGE Currency Exchange" width="96" height="67" />
          </span>
        }
      >
        <div class="xc-customer-bar">
          <Show when={!paired()}>
            <Show when={profile()}>
              {(p) => (
                <Show
                  when={p().signedIn ? (p() as { signedIn: true; name: string }) : undefined}
                  fallback={
                    <Show when={props.route !== '/app/sign-in' && props.route !== '/app/sign-up'}>
                      <a class="xc-customer-signin" href="/app/sign-in">
                        {tr('signIn')}
                      </a>
                    </Show>
                  }
                >
                  {(signedIn) => (
                    <>
                      <a
                        class="xc-customer-name"
                        href="/app/profile"
                        aria-current={props.route === '/app/profile' ? 'page' : undefined}
                      >
                        {tr('signedInAs', { name: signedIn().name })}
                      </a>
                      <a
                        class="xc-customer-member"
                        href="/app/member"
                        aria-current={props.route === '/app/member' ? 'page' : undefined}
                      >
                        {tr('member')}
                      </a>
                      <a
                        class="xc-customer-rate-alerts"
                        href="/app/rate-alerts"
                        aria-current={props.route === '/app/rate-alerts' ? 'page' : undefined}
                      >
                        {tr('rateAlerts')}
                      </a>
                    </>
                  )}
                </Show>
              )}
            </Show>
            <a
              class="xc-customer-notifications"
              href="/app/notifications"
              aria-current={props.route === '/app/notifications' ? 'page' : undefined}
            >
              {tr('notifications')}
            </a>
          </Show>
        </div>
        <main class="xc-shell-main xc-customer-main">{props.children}</main>
        <Show when={!paired()}>
          <nav class="xc-customer-nav" aria-label={tr('nav')}>
            <For each={NAV}>
              {(item) => (
                <a href={item.href} aria-current={current() === item.href ? 'page' : undefined}>
                  <svg class="xc-customer-nav-icon" viewBox="0 0 24 24" aria-hidden="true">
                    {ICON[item.key]()}
                  </svg>
                  <span class="xc-customer-nav-label">{tr(item.key)}</span>
                </a>
              )}
            </For>
          </nav>
        </Show>
      </AppShell>
    </div>
  );
}
