// Marks the field that is wrong, so the eye and the caret land on it (row FV).

/** The parts of a form control the marks use; a real element has them all. */
export interface Markable {
  classList?: { add(c: string): void; remove(c: string): void };
  setAttribute?(name: string, value: string): void;
  removeAttribute?(name: string): void;
}

interface Validatable {
  willValidate?: boolean;
  checkValidity?(): boolean;
}

/** The first control that fails its own rules, in document order, or null. */
export function firstInvalid<T extends Validatable>(elements: ArrayLike<T> | null | undefined): T | null {
  if (!elements) return null;
  for (let i = 0; i < elements.length; i++) {
    const el = elements[i];
    if (el && el.willValidate && typeof el.checkValidity === 'function' && !el.checkValidity()) {
      return el;
    }
  }
  return null;
}

/** Colours the field red and tells assistive technology it is invalid. */
export function markInvalid(el: Markable | null | undefined): void {
  if (!el) return;
  if (el.classList && typeof el.classList.add === 'function') el.classList.add('xc-invalid');
  if (typeof el.setAttribute === 'function') el.setAttribute('aria-invalid', 'true');
}

/** Undoes {@link markInvalid}. */
export function clearMark(el: Markable | null | undefined): void {
  if (!el) return;
  if (el.classList && typeof el.classList.remove === 'function') el.classList.remove('xc-invalid');
  if (typeof el.removeAttribute === 'function') el.removeAttribute('aria-invalid');
}
