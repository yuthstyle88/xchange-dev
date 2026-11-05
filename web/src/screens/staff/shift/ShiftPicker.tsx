import { createEffect, createSignal, For, Show, type Accessor, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { OpenShiftChoice, OpenShiftChoices } from '../../../shared/api/generated/open_shift_choices';
import { screenT } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import { en, th } from './i18n';
import './shift.css';

const TIME_FMT = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

export function formatShiftChoiceTime(isoOrTime: string): string {
  if (/^\d{2}:\d{2}(:\d{2})?$/.test(isoOrTime)) {
    return isoOrTime.slice(0, 5);
  }
  const d = new Date(isoOrTime);
  return Number.isNaN(d.getTime()) ? isoOrTime : TIME_FMT.format(d);
}

export function formatChoiceLabel(choice: OpenShiftChoice): string {
  return `${choice.name} · ${formatShiftChoiceTime(choice.startsAt)}–${formatShiftChoiceTime(choice.endsAt)}`;
}

export function shiftChosenOrUnneeded(
  choices: readonly OpenShiftChoice[] | OpenShiftChoices | null | undefined,
  value: string | null | undefined | (() => string | null | undefined),
): boolean {
  if (!choices) return false;
  const list: readonly OpenShiftChoice[] = 'choices' in choices ? choices.choices : choices;
  if (!list || list.length === 0) return false;
  if (list.length === 1) return true;
  const val = typeof value === 'function' ? value() : value;
  if (!val) return false;
  return list.some((c: OpenShiftChoice) => c.templateId === val);
}

export interface ShiftPickerProps {
  ctx: StaffCtx;
  value: Accessor<string | null>;
  onChange(id: string | null): void;
  api?: ApiClient;
  onChoices?(choices: OpenShiftChoice[]): void;
}

export function ShiftPicker(props: ShiftPickerProps): JSX.Element {
  const t = screenT('staff.shift', { th, en });
  const [choices, setChoices] = createSignal<OpenShiftChoice[]>([]);
  const [loaded, setLoaded] = createSignal(false);

  let seq = 0;
  const load = async (branch: string) => {
    const mine = ++seq;
    try {
      const api = props.api ?? new ApiClient();
      const res = await api.view<OpenShiftChoices>('open_shift_choices', { branch });
      if (mine !== seq) return;
      const raw = res.data;
      const list: OpenShiftChoice[] = Array.isArray(raw)
        ? raw
        : (raw?.choices ?? []);
      setChoices(list);
      props.onChoices?.(list);
      if (list.length === 1) {
        props.onChange(list[0].templateId);
      } else if (list.length === 0) {
        props.onChange(null);
      }
    } catch {
      if (mine !== seq) return;
      setChoices([]);
      props.onChoices?.([]);
      props.onChange(null);
    } finally {
      if (mine === seq) {
        setLoaded(true);
      }
    }
  };

  createEffect(() => {
    const branch = props.ctx.me.branchId;
    void load(branch);
  });

  return (
    <Show when={loaded()}>
      <div class="xc-shift-picker" data-testid="shift-picker">
        <Show
          when={choices().length > 0}
          fallback={
            <div class="xc-shift-picker-outside" data-testid="outside-shift-hours">
              {t('outsideShiftHours')}
            </div>
          }
        >
          <For each={choices()}>
            {(choice) => (
              <label
                class="xc-shift-choice-label"
                data-testid={`shift-choice-${choice.templateId}`}
              >
                <input
                  type="radio"
                  name="shift-choice"
                  value={choice.templateId}
                  checked={
                    choices().length === 1
                      ? (props.value() === null || props.value() === choice.templateId)
                      : props.value() === choice.templateId
                  }
                  onChange={() => props.onChange(choice.templateId)}
                />
                <span>{formatChoiceLabel(choice)}</span>
              </label>
            )}
          </For>
        </Show>
      </div>
    </Show>
  );
}
