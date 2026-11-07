import { For, Show, type JSX } from 'solid-js';
import { audience, lang, setLang, t } from './index';
import { NATIVE_NAME, SPOKEN, type Lang } from './languages';

export function LangSwitch(): JSX.Element {
  return (
    <Show
      when={audience() === 'customer'}
      fallback={
        <div role="group" class="xc-lang-switch">
          <For each={SPOKEN.staff}>
            {(l: Lang) => (
              <button
                type="button"
                onClick={() => setLang(l)}
                aria-pressed={lang() === l ? 'true' : 'false'}
                aria-label={t('shell.langSwitch', { language: NATIVE_NAME[l] })}
              >
                {l === 'th' ? 'ไทย' : 'EN'}
              </button>
            )}
          </For>
        </div>
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
