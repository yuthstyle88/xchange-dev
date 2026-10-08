import type { JSX } from 'solid-js';
import type { FieldBinding } from './form';

export function TextField(props: {
  field: FieldBinding;
  label: string;
  type?: string;
  autocomplete?: string;
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
