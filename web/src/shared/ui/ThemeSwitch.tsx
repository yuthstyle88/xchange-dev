import type { JSX } from 'solid-js';
import { t } from '../i18n';
import { themeMode, toggleThemeMode } from './theme-mode';

/** One button that flips light ↔ dark; the icon shows the mode it switches to. */
export function ThemeSwitch(): JSX.Element {
  return (
    <button
      type="button"
      class="xc-theme-switch"
      aria-pressed={themeMode() === 'dark' ? 'true' : 'false'}
      aria-label={t('shell.themeSwitch')}
      title={t('shell.themeSwitch')}
      onClick={() => toggleThemeMode()}
    >
      <span aria-hidden="true">{themeMode() === 'dark' ? '☀' : '☾'}</span>
    </button>
  );
}
