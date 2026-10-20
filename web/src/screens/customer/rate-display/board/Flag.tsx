// P01 v2 flag (rate-display.md §9.7): the vendored flag image, or a code box when there is none or it fails to load.
import { createSignal, Show, type JSX } from 'solid-js';
import { flagUrl } from './flag-code';
import './flag.css';

export interface FlagProps {
  currency: string;
}

export function Flag(props: FlagProps): JSX.Element {
  const [failedCurrency, setFailedCurrency] = createSignal<string | null>(null);
  const activeUrl = () => {
    if (failedCurrency() === props.currency) return null;
    return flagUrl(props.currency);
  };

  return (
    <Show
      when={activeUrl()}
      fallback={<span class="xc-rb-code-box">{props.currency}</span>}
    >
      {(url) => (
        <img
          class="xc-rb-flag"
          src={url()}
          alt=""
          onError={() => setFailedCurrency(props.currency)}
        />
      )}
    </Show>
  );
}

