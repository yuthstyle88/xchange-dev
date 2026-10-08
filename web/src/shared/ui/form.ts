import { createSignal, type Accessor } from 'solid-js';
import { ApiError } from '../api/client';
import { errorText } from '../i18n';

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

interface ErrorState {
  code?: string;
  message: string;
}

interface InternalField {
  binding: FieldBinding;
  element?: HTMLInputElement;
  isClientError: boolean;
  setInvalid: (v: boolean) => void;
  setErrState: (v: ErrorState | undefined) => void;
}

export function useForm(): Form {
  const [formErr, setFormErr] = createSignal<ErrorState | undefined>(undefined);
  const fields = new Map<string, InternalField>();

  function field(name: string): FieldBinding {
    const existing = fields.get(name);
    if (existing) return existing.binding;

    const [invalid, setInvalid] = createSignal(false);
    const [errState, setErrState] = createSignal<ErrorState | undefined>(undefined);
    const error: Accessor<string | undefined> = () => {
      const e = errState();
      if (!e) return undefined;
      return e.code ? errorText(e.code, e.message) : e.message;
    };
    const errorId = `${name}-error`;

    const internal: InternalField = {
      element: undefined,
      isClientError: false,
      setInvalid,
      setErrState,
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
                internal.setErrState(undefined);
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
        f.setErrState(undefined);
        if (f.element) {
          f.element.removeAttribute('aria-invalid');
        }
      }

      let firstFailed: HTMLInputElement | undefined;
      for (const f of fields.values()) {
        if (f.element && f.element.willValidate && !f.element.checkValidity()) {
          f.isClientError = true;
          f.setInvalid(true);
          f.setErrState({ message: f.element.validationMessage });
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
              target.setErrState({ code: err.code, message: err.message });
              if (target.element) {
                target.element.setAttribute('aria-invalid', 'true');
                target.element.focus();
              }
            } else {
              setFormErr({ code: err.code, message: err.message });
            }
          } else {
            setFormErr({ code: err.code, message: err.message });
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
    const e = formErr();
    if (!e) return undefined;
    return e.code ? errorText(e.code, e.message) : e.message;
  }

  return {
    field,
    submit,
    errorFor,
    formError,
  };
}
