import { createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { t } from '../i18n';
import type { FieldBinding } from './form';

export function PasswordField(props: {
  field: FieldBinding;
  label: string;
  autocomplete?: string;
  required?: boolean;
  maxLength?: number;
}): JSX.Element {
  const id = `field-${props.field.name}`;
  const [visible, setVisible] = createSignal(false);
  let inputEl: HTMLInputElement | undefined;
  let wrapperEl: HTMLDivElement | undefined;

  function toggle(e: MouseEvent) {
    e.preventDefault();
    const next = !visible();
    setVisible(next);
    if (inputEl) {
      inputEl.type = next ? 'text' : 'password';
    }
  }

  onMount(() => {
    const form = inputEl?.form ?? wrapperEl?.closest('form');
    if (!form) return;

    const onSubmit = () => {
      setVisible(false);
      if (inputEl) {
        inputEl.type = 'password';
      }
    };
    form.addEventListener('submit', onSubmit);
    onCleanup(() => {
      form.removeEventListener('submit', onSubmit);
    });
  });

  return (
    <div class="xc-field">
      <label for={id}>{props.label}</label>
      <div class="xc-password" ref={wrapperEl}>
        <input
          id={id}
          name={props.field.name}
          type="password"
          autocomplete={props.autocomplete}
          required={props.required ? true : undefined}
          maxlength={props.maxLength}
          ref={(el) => {
            inputEl = el;
            props.field.ref(el);
          }}
          aria-invalid={props.field.invalid() ? 'true' : undefined}
          aria-describedby={props.field.invalid() ? props.field.errorId : undefined}
        />
        <button
          type="button"
          class="xc-reveal"
          onClick={toggle}
          aria-label={visible() ? t('field.password.hide') : t('field.password.show')}
          aria-pressed={visible() ? 'true' : 'false'}
        >
          <span class="xc-reveal-text">
            {visible() ? t('field.password.hide') : t('field.password.show')}
          </span>
        </button>
      </div>
      {props.field.invalid() && props.field.error() && (
        <span id={props.field.errorId} role="alert">
          {props.field.error()}
        </span>
      )}
    </div>
  );
}
