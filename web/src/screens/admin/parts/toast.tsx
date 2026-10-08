// V2-parts (#476): toast notification (ticket §7, design §12.2)

import { createSignal, type JSX } from 'solid-js';

const [text, setText] = createSignal<string>('');
let timer: ReturnType<typeof setTimeout> | undefined;

export function toast(message: string): void {
  if (timer !== undefined) {
    clearTimeout(timer);
  }
  setText(message);
  timer = setTimeout(() => {
    setText('');
    timer = undefined;
  }, 5000);
}

export function Toast(): JSX.Element {
  return (
    <div
      role="status"
      aria-live="polite"
      class="toast"
      hidden={text() === ''}
    >
      {text()}
    </div>
  );
}
