// RD-3 full screen button (design rate-display.md D8).
import { createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';

export interface FullscreenButtonProps {
  label: string;
}

export function FullscreenButton(props: FullscreenButtonProps): JSX.Element {
  const isSupported =
    typeof document !== 'undefined' &&
    typeof document.documentElement?.requestFullscreen === 'function';

  if (!isSupported) {
    return null;
  }

  const [isFullscreen, setIsFullscreen] = createSignal(
    Boolean(typeof document !== 'undefined' && document.fullscreenElement),
  );

  const handleChange = () => {
    setIsFullscreen(Boolean(document.fullscreenElement));
  };

  onMount(() => {
    document.addEventListener('fullscreenchange', handleChange);
  });

  onCleanup(() => {
    document.removeEventListener('fullscreenchange', handleChange);
  });

  const handleClick = () => {
    void document.documentElement.requestFullscreen();
  };

  return (
    <Show when={!isFullscreen()}>
      <button
        type="button"
        class="xc-rate-display-fullscreen-btn"
        aria-label={props.label}
        onClick={handleClick}
      >
        {props.label}
      </button>
    </Show>
  );
}
