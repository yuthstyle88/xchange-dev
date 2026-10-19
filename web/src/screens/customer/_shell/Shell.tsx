// APP-7 (#998) the customer app shell (design customer-app.md §4.5; contract issuecomment-6137548682; LK-1 ruling
// issuecomment-6137816294). It composes AppShell, as AdminShell does, and adds the customer's bottom navigation and
// sign-in state. AppShell's header carries LangSwitch, which shows TH / EN / 中文 for the customer audience.
// A screen never reads my_profile or the language itself.
import { createResource, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { MyProfileData } from '../../../shared/api/generated/my_profile';
import { screenT } from '../../../shared/i18n';
import { AppShell } from '../../../shared/ui/AppShell';
import * as table from './i18n';
import './shell.css';

const tr = screenT('customer._shell', table);

export type CustomerRoute =
  | '/app'
  | '/app/history'
  | '/app/branches'
  | '/app/journey'
  | '/app/book'
  | '/app/bookings'
  | '/app/assistant'
  | '/app/rate-alerts'
  | '/app/notifications'
  | '/app/member'
  | '/app/profile';

type NavKey = 'home' | 'branches' | 'bookings' | 'history' | 'assistant';

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
  '/app/history': '/app/history',
  '/app/assistant': '/app/assistant',
  '/app/rate-alerts': undefined,
  '/app/notifications': undefined,
  '/app/member': undefined,
  '/app/profile': undefined,
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

export function CustomerShell(props: { route: CustomerRoute; children?: JSX.Element }): JSX.Element {
  const [profile] = createResource(loadProfile);
  const current = () => NAV_OF[props.route];

  return (
    <AppShell title={tr('title')} ownsMain staffSession={false}>
      <div class="xc-customer-bar">
        <Show when={profile()}>
          {(p) => (
            <Show
              when={p().signedIn ? (p() as { signedIn: true; name: string }) : undefined}
              fallback={
                <a class="xc-customer-signin" href="/app/sign-in">
                  {tr('signIn')}
                </a>
              }
            >
              {(signedIn) => (
                <span class="xc-customer-name">{tr('signedInAs', { name: signedIn().name })}</span>
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
      </div>
      <main class="xc-shell-main xc-customer-main">{props.children}</main>
      <nav class="xc-customer-nav" aria-label={tr('nav')}>
        <For each={NAV}>
          {(item) => (
            <a href={item.href} aria-current={current() === item.href ? 'page' : undefined}>
              {tr(item.key)}
            </a>
          )}
        </For>
      </nav>
    </AppShell>
  );
}
