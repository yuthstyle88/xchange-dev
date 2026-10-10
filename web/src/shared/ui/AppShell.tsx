import { Show, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { t } from '../i18n';
import type { Key } from '../i18n/th';
import { sharedSession } from '../session/session';
import { AccountBar } from './AccountBar';
import { VersionFooter } from './VersionFooter';
import './shell.css';

export type AppShellLook = 'devices' | 'kiosk' | 'admin';

export function AppShell(props: {
  title: string;
  nav?: JSX.Element;
  children?: JSX.Element;
  ownsMain?: boolean;
  look?: AppShellLook;
  banner?: JSX.Element;
}): JSX.Element {
  const session = sharedSession();

  return (
    <div class={props.look ? `xc-shell xc-${props.look}` : 'xc-shell'}>
      {props.banner ? <div class="xc-shell-banner">{props.banner}</div> : null}
      <AccountBar session={session} />
      <header class="xc-shell-header">
        {props.nav ? <div class="xc-shell-nav">{props.nav}</div> : null}
        <h1 class="xc-shell-title">{props.title}</h1>
        <LangSwitch />
      </header>
      <Show when={session.ended()}>
        <p role="alert" class="xc-shell-alert">{t(`shell.session.${session.ended()!}` as Key)}</p>
      </Show>
      <fieldset
        class="xc-lock"
        disabled={session.ended() !== undefined}
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
