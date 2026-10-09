import { createSignal, onMount, type JSX } from 'solid-js';

interface ReadyData {
  version: string;
  build: string;
  channel: string;
  builtAt: string;
}

export function VersionFooter(): JSX.Element {
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
  );
}
