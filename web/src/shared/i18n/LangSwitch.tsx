import type { JSX } from 'solid-js';
import { lang, setLang, t } from './index';

export function LangSwitch(): JSX.Element {
  return (
    <button
      type="button"
      onClick={() => setLang(lang() === 'th' ? 'en' : 'th')}
      aria-label={t('shell.langSwitch', { language: lang() === 'th' ? 'English' : 'ไทย' })}
    >
      {lang() === 'th' ? 'EN' : 'TH'}
    </button>
  );
}
