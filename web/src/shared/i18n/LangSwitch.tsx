import { For, Show, type JSX } from 'solid-js';
import { audience, lang, setLang, t } from './index';
import { NATIVE_NAME, SPOKEN, type Lang } from './languages';

export function LangSwitch(): JSX.Element {
  return (
    <Show
      when={audience() === 'customer'}
      fallback={
        <button
          type="button"
          onClick={() => setLang(lang() === 'th' ? 'en' : 'th')}
          aria-label={t('shell.langSwitch', {
            language: lang() === 'th' ? 'English' : 'ไทย',
          })}
        >
          {lang() === 'th' ? 'EN' : 'TH'}
        </button>
      }
    >
      <For each={SPOKEN.customer}>
        {(l: Lang) => (
          <button
            type="button"
            onClick={() => setLang(l)}
            aria-pressed={lang() === l ? 'true' : 'false'}
            aria-label={t('shell.langSwitch', { language: NATIVE_NAME[l] })}
          >
            {l === 'zh' ? '中文' : l.toUpperCase()}
          </button>
        )}
      </For>
    </Show>
  );
}
