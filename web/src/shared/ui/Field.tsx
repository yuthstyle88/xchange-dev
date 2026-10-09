import type { JSX } from 'solid-js';
import { Show } from 'solid-js';
import './card.css';

export interface FieldProps {
  label: string;
  hint?: string;
  unit?: string;
  class?: string;
  children?: JSX.Element;
}

export function Field(props: FieldProps): JSX.Element {
  const fieldClass = () =>
    props.class ? `form-field ${props.class}` : 'form-field';
  return (
    <label class={fieldClass()}>
      <span class="form-label">
        {props.label}
        <Show when={props.unit}>
          <span class="form-unit">{props.unit}</span>
        </Show>
      </span>
      <Show when={props.hint}>
        <span class="form-hint">{props.hint}</span>
      </Show>
      {props.children}
    </label>
  );
}

export interface CheckFieldProps {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  required?: boolean;
  name?: string;
  id?: string;
  class?: string;
}

export function CheckField(props: CheckFieldProps): JSX.Element {
  const checkClass = () =>
    props.class ? `check-field ${props.class}` : 'check-field';
  return (
    <label class={checkClass()}>
      <input
        type="checkbox"
        id={props.id}
        name={props.name}
        checked={props.checked}
        disabled={props.disabled}
        required={props.required}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
      />
      <span class="check-text">
        <span class="form-label">{props.label}</span>
        <Show when={props.hint}>
          <span class="form-hint">{props.hint}</span>
        </Show>
      </span>
    </label>
  );
}
