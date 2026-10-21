import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import './desk.css';
import * as screenI18n from './i18n';
import type { DeskStep, FrameStore } from './steps';

interface TaskMod<S> {
  createTask?(props: { store: S }): {
    nav(): JSX.Element;
    panel(): JSX.Element;
    isOpen(): boolean;
  };
}

const taskMods = import.meta.glob<TaskMod<any>>('./steps/refunds/task.tsx', { eager: true });
const taskMod = taskMods['./steps/refunds/task.tsx'];

export function DeskPage<S extends FrameStore>(props: {
  store: S;
  steps: DeskStep<S>[];
  aside?: JSX.Element;
}): JSX.Element {
  const t = screenT('desk', screenI18n);
  const current = () => props.steps.find((s) => s.index === props.store.step());
  const task = taskMod?.createTask?.({ store: props.store });
  const withEntry = props.store as S & {
    entry?: () => { banner: () => string; actions?: () => JSX.Element } | undefined;
  };

  return (
    <div class="desk-page">
      <Show when={withEntry.entry?.()}>
        {(entry) => (
          <div class="desk-entry-bar" data-desk-entry-bar="">
            <span>{entry().banner()}</span>
            {entry().actions?.()}
          </div>
        )}
      </Show>
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
        {task?.nav()}
      </nav>

      <div class="desk-body">
        <section class="panel" id="panel">
          <Show
            when={task?.isOpen()}
            fallback={
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
            }
          >
            {task?.panel()}
          </Show>
        </section>

        <Show when={props.aside}>
          <aside class="desk-aside">{props.aside}</aside>
        </Show>
      </div>
    </div>
  );
}
