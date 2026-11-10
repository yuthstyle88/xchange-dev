import { createEffect, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { sharedSession, type SessionStore } from '../../shared/session/session';
import { AppShell } from '../../shared/ui/AppShell';
import { PermissionsUnavailable } from '../../shared/ui/PermissionsUnavailable';
import {
  hasVisibleSettings,
  visibleSettingsLinks,
  type SettingsGroup,
  type SettingsLinkKey,
} from '../../shared/ui/staff-nav';
import * as table from './i18n';
import './staff-settings.css';

const t = screenT('staff-settings', table);

const GROUPS: readonly SettingsGroup[] = ['cash', 'alerts'];

const DESC_KEY: Record<SettingsLinkKey, keyof typeof table.th> = {
  cashLimits: 'cashLimitsDesc',
  cashPolicy: 'cashPolicyDesc',
  alertSettings: 'alertSettingsDesc',
  rateAlertSettings: 'rateAlertSettingsDesc',
};

export interface StaffSettingsProps {
  session?: SessionStore;
  navigate?: (url: string) => void;
}

export function StaffSettings(props: StaffSettingsProps = {}): JSX.Element {
  const session = () => props.session ?? sharedSession();
  const me = () => session().me();
  const meError = () => (me() === undefined ? (session().meError?.() ?? null) : null);

  createEffect(() => {
    const err = meError();
    if (err && err.status === 401 && err.code === 'signIn') {
      const nav = props.navigate ?? ((url: string) => window.location.assign(url));
      nav('/login?next=%2Fstaff%2Fsettings');
    }
  });

  const linksFor = (group: SettingsGroup) =>
    visibleSettingsLinks(me()?.permissions ?? []).filter((item) => item.group === group);

  return (
    <AppShell title={t('title')}>
      <div class="xc-staff-settings">
        <Show
          when={me()}
          fallback={
            <Show
              when={meError()}
              fallback={
                <p class="xc-staff-settings-state" data-testid="staff-settings-loading">
                  {t('loading')}
                </p>
              }
            >
              {(err) => <PermissionsUnavailable error={err()} onRetry={() => session().retryMe?.()} />}
            </Show>
          }
        >
          {(user) => (
            <Show
              when={hasVisibleSettings(user().permissions)}
              fallback={
                <p class="xc-staff-settings-state" data-testid="staff-settings-empty">
                  {t('noAccess')}
                </p>
              }
            >
              <For each={GROUPS}>
                {(group) => (
                  <Show when={linksFor(group).length > 0}>
                    <section
                      class="xc-staff-settings-group"
                      data-settings-group={group}
                      aria-labelledby={`staff-settings-group-${group}`}
                    >
                      <h2 id={`staff-settings-group-${group}`}>{t(group)}</h2>
                      <div class="xc-staff-settings-links">
                        <For each={linksFor(group)}>
                          {(item) => (
                            <a class="xc-staff-settings-link" href={item.href}>
                              <span class="xc-staff-settings-link-title">{t(item.key)}</span>
                              <span class="xc-staff-settings-link-desc">{t(DESC_KEY[item.key])}</span>
                            </a>
                          )}
                        </For>
                      </div>
                    </section>
                  </Show>
                )}
              </For>
            </Show>
          )}
        </Show>
      </div>
    </AppShell>
  );
}
