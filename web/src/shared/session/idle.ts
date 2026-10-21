export const TABLET_IDLE_MS = 120_000;

export function idleTimer(ms: number, onIdle: () => void): { touch(): void; dispose(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;

  const touch = (): void => {
    if (disposed) return;
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(onIdle, ms);
  };

  const dispose = (): void => {
    disposed = true;
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (typeof document !== 'undefined') {
      document.removeEventListener('pointerdown', touch);
      document.removeEventListener('keydown', touch);
    }
  };

  if (typeof document !== 'undefined') {
    document.addEventListener('pointerdown', touch);
    document.addEventListener('keydown', touch);
  }
  touch();

  return { touch, dispose };
}
