// P01 v2 split-flap price cell (rate-display.md §9.2, §9.6): one span per character; only the characters that
// changed since the previous text flip; an arrow and an up/down glow when `direction` is not 0.
import { For, Show, createMemo, type JSX } from 'solid-js';
import './flap-price.css';

export interface FlapPriceProps {
  /** The formatted text (formatBandRate), e.g. `35.42` or `—`. */
  text: string;
  /** 1 when the rate rose since the previous text, -1 when it fell, 0 for no arrow (computed by the row). */
  direction: -1 | 0 | 1;
}

interface FlapCell {
  ch: string;
  sep: boolean;
  flip: boolean;
  delay: string;
}

export function FlapPrice(props: FlapPriceProps): JSX.Element {
  let prev: string | undefined;

  const cells = createMemo<FlapCell[]>(() => {
    const next = props.text;
    const old = prev;
    prev = next;
    return Array.from(next, (ch, i) => ({
      ch,
      sep: ch === '.' || ch === ',',
      flip: old !== undefined && old[old.length - next.length + i] !== ch,
      delay: `${i * 45}ms`,
    }));
  });

  return (
    <span
      class="xc-rb-price"
      classList={{
        'xc-rb-up': props.direction > 0,
        'xc-rb-down': props.direction < 0,
      }}
    >
      <Show when={props.direction !== 0}>
        <em>{props.direction > 0 ? '▲' : '▼'}</em>
      </Show>
      <For each={cells()}>
        {(cell) => (
          <span
            class="xc-rb-ch"
            classList={{
              'xc-rb-sep': cell.sep,
              'xc-rb-flip': cell.flip,
            }}
            style={cell.flip ? { '--d': cell.delay } : undefined}
          >
            {cell.ch}
          </span>
        )}
      </For>
    </span>
  );
}
