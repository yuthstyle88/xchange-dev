import { Show, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { t } from '../i18n';
import type { Key } from '../i18n/th';
import { sharedSession } from '../session/session';
import { AccountBar } from './AccountBar';
import { VersionFooter } from './VersionFooter';

export function AppShell(props: {
  title: string;
  nav?: JSX.Element;
  children?: JSX.Element;
  ownsMain?: boolean;
}): JSX.Element {
  const session = sharedSession();

  return (
    <div>
      <header>
        {props.nav}
        <h1>{props.title}</h1>
        <LangSwitch />
        <AccountBar session={session} />
      </header>
      <Show when={session.ended()}>
        <p role="alert">{t(`shell.session.${session.ended()!}` as Key)}</p>
      </Show>
      <fieldset
        class="xc-lock"
        disabled={session.ended() !== undefined}
        style="border: 0; padding: 0; margin: 0; min-width: 0"
      >
        {props.ownsMain ? (
          <div class="xc-content">{props.children}</div>
        ) : (
          <main>{props.children}</main>
        )}
      </fieldset>
      <VersionFooter />
    </div>
  );
}
