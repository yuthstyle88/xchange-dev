import { createSignal, type Accessor } from 'solid-js';
import { ApiError } from '../api/client';

export interface FieldBinding {
  name: string;
  ref: (el: HTMLInputElement) => void;
  invalid: Accessor<boolean>;
  error: Accessor<string | undefined>;
  errorId: string;
}

export interface Form {
  field(name: string): FieldBinding;
  submit(fn: () => Promise<unknown>): (e?: Event) => Promise<void>;
  errorFor(name: string): string | undefined;
  formError(): string | undefined;
}

interface InternalField {
  binding: FieldBinding;
  element?: HTMLInputElement;
  isClientError: boolean;
  setInvalid: (v: boolean) => void;
  setError: (v: string | undefined) => void;
}

export function useForm(): Form {
  const [formErr, setFormErr] = createSignal<string | undefined>(undefined);
  const fields = new Map<string, InternalField>();

  function field(name: string): FieldBinding {
    const existing = fields.get(name);
    if (existing) return existing.binding;

    const [invalid, setInvalid] = createSignal(false);
    const [error, setError] = createSignal<string | undefined>(undefined);
    const errorId = `${name}-error`;

    const internal: InternalField = {
      element: undefined,
      isClientError: false,
      setInvalid,
      setError,
      binding: {
        name,
        invalid,
        error,
        errorId,
        ref: (el: HTMLInputElement) => {
          if (!el || internal.element === el) return;
          internal.element = el;

          const onInputOrChange = () => {
            if (internal.isClientError && internal.element) {
              if (internal.element.validity.valid) {
                internal.isClientError = false;
                internal.setInvalid(false);
                internal.setError(undefined);
                internal.element.removeAttribute('aria-invalid');
                internal.element.removeAttribute('aria-describedby');
              }
            }
          };

          el.addEventListener('input', onInputOrChange);
          el.addEventListener('change', onInputOrChange);
        },
      },
    };

    fields.set(name, internal);
    return internal.binding;
  }

  function submit(fn: () => Promise<unknown>): (e?: Event) => Promise<void> {
    return async (e?: Event) => {
      if (e && typeof e.preventDefault === 'function') {
        e.preventDefault();
      }

      setFormErr(undefined);
      for (const f of fields.values()) {
        f.isClientError = false;
        f.setInvalid(false);
        f.setError(undefined);
        if (f.element) {
          f.element.removeAttribute('aria-invalid');
        }
      }

      let firstFailed: HTMLInputElement | undefined;
      for (const f of fields.values()) {
        if (f.element && f.element.willValidate && !f.element.checkValidity()) {
          f.isClientError = true;
          f.setInvalid(true);
          f.setError(f.element.validationMessage);
          f.element.setAttribute('aria-invalid', 'true');
          if (!firstFailed) {
            firstFailed = f.element;
          }
        }
      }

      if (firstFailed) {
        firstFailed.focus();
        return;
      }

      try {
        await fn();
      } catch (err) {
        if (err instanceof ApiError) {
          if (err.field) {
            const target = fields.get(err.field);
            if (target) {
              target.isClientError = false;
              target.setInvalid(true);
              target.setError(err.message);
              if (target.element) {
                target.element.setAttribute('aria-invalid', 'true');
                target.element.focus();
              }
            } else {
              setFormErr(err.message);
            }
          } else {
            setFormErr(err.message);
          }
        } else {
          throw err;
        }
      }
    };
  }

  function errorFor(name: string): string | undefined {
    return fields.get(name)?.binding.error();
  }

  function formError(): string | undefined {
    return formErr();
  }

  return {
    field,
    submit,
    errorFor,
    formError,
  };
}
