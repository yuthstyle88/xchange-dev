// Light / dark mode for staff pages (docs/design/xchange-one-look.md §2.6).
// The choice lives on <html data-theme>, so every .xc-theme surface on the page follows it;
// customer pages and print stay light (xc-theme.css scopes the dark values).
import { createSignal } from 'solid-js';

export type ThemeMode = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'xc-theme-mode';

function stored(): ThemeMode | null {
  try {
    const v = globalThis.localStorage?.getItem(THEME_STORAGE_KEY);
    return v === 'light' || v === 'dark' ? v : null;
  } catch {
    return null;
  }
}

function systemPrefersDark(): boolean {
  try {
    return globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches === true;
  } catch {
    return false;
  }
}

/** A saved choice wins; with none, follow the device setting. */
export function initialThemeMode(): ThemeMode {
  return stored() ?? (systemPrefersDark() ? 'dark' : 'light');
}

function apply(mode: ThemeMode): void {
  if (typeof document !== 'undefined') document.documentElement.dataset.theme = mode;
}

const [mode, setModeSignal] = createSignal<ThemeMode>(initialThemeMode());
apply(mode());

export const themeMode = mode;

export function setThemeMode(next: ThemeMode): void {
  setModeSignal(next);
  apply(next);
  try {
    globalThis.localStorage?.setItem(THEME_STORAGE_KEY, next);
  } catch {
    // Private mode or blocked storage: the choice holds for this page only.
  }
}

export function toggleThemeMode(): void {
  setThemeMode(mode() === 'dark' ? 'light' : 'dark');
}
