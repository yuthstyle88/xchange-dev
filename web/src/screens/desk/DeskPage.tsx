import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import './desk.css';
import * as screenI18n from './i18n';
import type { DeskStep, FrameStore } from './steps';

export function DeskPage<S extends FrameStore>(props: {
  store: S;
  steps: DeskStep<S>[];
  aside?: JSX.Element;
}): JSX.Element {
  const t = screenT('desk', screenI18n);
  const current = () => props.steps.find((s) => s.index === props.store.step());

  return (
    <div class="desk-page">
      <nav class="desk-nav">
        <ol class="steps" aria-label={t('stepsLabel')}>
          <For each={props.steps}>
            {(step) => (
              <li
                aria-current={
                  step.index === props.store.step() ? 'step' : undefined
                }
              >
                {String(step.index + 1).padStart(2, '0')}&nbsp;{step.label()}
              </li>
            )}
          </For>
        </ol>
      </nav>

      <div class="desk-body">
        <section class="panel" id="panel">
          <Show
            when={current()}
            fallback={<div class="desk-empty">{t('empty')}</div>}
          >
            {(step) => (
              <div class="desk-step-content">
                {step().component({ store: props.store })}
              </div>
            )}
          </Show>
        </section>

        <Show when={props.aside}>
          <aside class="desk-aside">{props.aside}</aside>
        </Show>
      </div>
    </div>
  );
}
