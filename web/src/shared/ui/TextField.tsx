import type { JSX } from 'solid-js';
import type { FieldBinding } from './form';

export function TextField(props: {
  field: FieldBinding;
  label: string;
  type?: string;
  autocomplete?: string;
  required?: boolean;
  pattern?: string;
  min?: string;
  max?: string;
  step?: string;
  inputMode?: 'text' | 'decimal' | 'numeric' | 'email' | 'tel' | 'search' | 'url' | 'none';
  maxLength?: number;
}): JSX.Element {
  const id = `field-${props.field.name}`;

  return (
    <div>
      <label for={id}>{props.label}</label>
      <input
        id={id}
        name={props.field.name}
        type={props.type ?? 'text'}
        autocomplete={props.autocomplete}
        required={props.required ? true : undefined}
        pattern={props.pattern}
        min={props.min}
        max={props.max}
        step={props.step}
        inputmode={props.inputMode}
        maxlength={props.maxLength}
        ref={props.field.ref}
        aria-invalid={props.field.invalid() ? 'true' : undefined}
        aria-describedby={props.field.invalid() ? props.field.errorId : undefined}
      />
      {props.field.invalid() && props.field.error() && (
        <span id={props.field.errorId} role="alert">
          {props.field.error()}
        </span>
      )}
    </div>
  );
}
