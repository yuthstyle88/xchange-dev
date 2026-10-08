import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { t } from '../i18n';
import type { Key } from '../i18n/th';
import { sharedSession } from '../session/session';
import { AccountBar } from './AccountBar';

interface ReadyData {
  version: string;
  build: string;
  channel: string;
  builtAt: string;
}

export function AppShell(props: {
  title: string;
  nav?: JSX.Element;
  children?: JSX.Element;
  ownsMain?: boolean;
}): JSX.Element {
  const [ready, setReady] = createSignal<ReadyData | undefined>(undefined);
  const session = sharedSession();

  onMount(async () => {
    try {
      const res = await fetch('/health/ready');
      const data = await res.json();
      if (
        data &&
        typeof data === 'object' &&
        data.version &&
        data.build &&
        data.channel &&
        data.builtAt
      ) {
        setReady({
          version: String(data.version),
          build: String(data.build),
          channel: String(data.channel),
          builtAt: String(data.builtAt),
        });
      }
    } catch {
      // ignore offline/unreadable
    }
  });

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
      <footer>
        {ready() && (
          <div>
            <span>
              v{ready()!.version} · {ready()!.build.slice(0, 7)} · {ready()!.channel}
            </span>{' '}
            <span>{ready()!.builtAt}</span>
          </div>
        )}
      </footer>
    </div>
  );
}
