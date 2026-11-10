import { createEffect, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { sharedSession, type SessionStore } from '../../shared/session/session';
import '../../shared/ui/card.css';
import './desk.css';
import { CounterUnlock } from './CounterUnlock';
import * as screenI18n from './i18n';
import { LowCashBanner } from './LowCashBanner';
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
  shift?: { open(): boolean; Panel(): JSX.Element };
  session?: SessionStore;
}): JSX.Element {
  const t = screenT('desk', screenI18n);
  const current = () => props.steps.find((s) => s.index === props.store.step());
  const task = taskMod?.createTask?.({ store: props.store });
  const shiftOpen = () => (props.shift ? props.shift.open() : true);
  const sessionProp = props.session;
  const storeAny = props.store as any;
  const session = () => sessionProp ?? (storeAny?.path ? sharedSession() : undefined);
  const canBackdate = () => session()?.me()?.permissions?.includes('exchangeBackdate') ?? false;
  const canUnlockCounter = () => session()?.me()?.permissions?.includes('counterUnlock') ?? false;
  const withEntry = props.store as S & {
    entry?: () => { banner: () => string; actions?: () => JSX.Element } | undefined;
    lowCash?: { low(): string[]; reload(): Promise<void> };
  };
  let wasOpen = false;
  createEffect(() => {
    const open = shiftOpen();
    if (open && !wasOpen) {
      void withEntry.lowCash?.reload();
    }
    wasOpen = open;
  });

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
      <Show when={shiftOpen() && withEntry.lowCash}>
        {(lc) => <LowCashBanner lowCash={lc()} />}
      </Show>
      <nav class="desk-nav">
        <Show when={shiftOpen()}>
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
        </Show>
        <Show when={canBackdate()}>
          <a href="/desk/backdated" data-desk-backdated-link="">
            {t('backdatedLink')}
          </a>
        </Show>
        <CounterUnlock branchId={session()?.me()?.branch ?? undefined} canIssue={canUnlockCounter()} />
        {task?.nav()}
      </nav>

      <div class="desk-body">
        <section class="panel card" id="panel">
          <Show
            when={task?.isOpen()}
            fallback={
              <Show when={shiftOpen()} fallback={props.shift?.Panel()}>
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
              </Show>
            }
          >
            {task?.panel()}
          </Show>
        </section>

        <Show when={shiftOpen() && props.aside}>
          <aside class="desk-aside card">{props.aside}</aside>
        </Show>
      </div>
    </div>
  );
}
