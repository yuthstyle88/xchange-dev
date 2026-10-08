import { createSignal, onMount, type JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
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
}): JSX.Element {
  const [ready, setReady] = createSignal<ReadyData | undefined>(undefined);

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
        <h1>{props.title}</h1>
        {props.nav}
        <LangSwitch />
        <AccountBar session={sharedSession()} />
      </header>
      <main>{props.children}</main>
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
