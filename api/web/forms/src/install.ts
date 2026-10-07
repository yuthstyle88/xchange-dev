// Wires the helper into a page: repairs patterns, wraps password fields, points server
// refusals at their field, and stops a form with a wrong field from submitting (row FV).

import { fixPattern } from './patterns';
import { clearMark, firstInvalid, markInvalid } from './marks';
import { REFUSALS, fieldFor } from './refusals';
import {
  REVEAL_ATTR,
  REVEAL_CLASS,
  REVEALABLE_ATTR,
  WRAPPER_CLASS,
  concealAll,
  syncReveal,
  wrapPasswordField,
} from './password-field';
import { CSS, STYLE_ID } from './style';

function addStyle(doc: Document): void {
  if (doc.getElementById(STYLE_ID)) return;
  const style = doc.createElement('style');
  style.id = STYLE_ID;
  style.textContent = CSS;
  (doc.head || doc.documentElement)?.appendChild(style);
}

function repairPatterns(doc: Document): void {
  for (const el of doc.querySelectorAll<HTMLInputElement>('input[pattern]')) {
    const pattern = el.getAttribute('pattern');
    const fixed = fixPattern(pattern);
    if (fixed !== null && fixed !== pattern) el.setAttribute('pattern', fixed);
  }
}

function wrapPasswordFields(doc: Document): void {
  for (const el of doc.querySelectorAll<HTMLInputElement>('input[type="password"]')) {
    if (!el.hasAttribute(REVEALABLE_ATTR)) wrapPasswordField(doc, el);
  }
}

/** Removes an eye whose field is gone, and the wrapper with it when nothing else is left. */
function sweepOrphans(doc: Document): void {
  for (const button of doc.querySelectorAll(`[${REVEAL_ATTR}]`)) {
    const field = button.previousElementSibling;
    if (field && doc.contains(field) && field.hasAttribute(REVEALABLE_ATTR)) continue;
    const wrapper = button.parentElement;
    button.remove();
    if (wrapper && wrapper.classList.contains(WRAPPER_CLASS) && wrapper.childElementCount === 0) {
      wrapper.remove();
    }
  }
}

function pointAtField(doc: Document, errorEl: Element, errorId: string): void {
  const text = errorEl.textContent || '';
  const fieldId = fieldFor(errorId, text);
  if (!fieldId) return;
  const field = doc.getElementById(fieldId) as HTMLInputElement | null;
  if (!field) return;
  markInvalid(field);
  if (typeof field.setCustomValidity === 'function') field.setCustomValidity(text.trim());
  if (typeof field.focus === 'function') field.focus();
  if (typeof field.reportValidity === 'function') field.reportValidity();
}

/** Installs the helper on `doc`. Called once per page; safe against pages that re-render. */
export function install(doc: Document | null | undefined): void {
  if (!doc) return;
  addStyle(doc);

  const watchedErrors = new Set<Element>();
  function watchRefusals(): void {
    for (const errorId of Object.keys(REFUSALS)) {
      const el = doc!.getElementById(errorId);
      if (!el || watchedErrors.has(el)) continue;
      watchedErrors.add(el);
      if (typeof MutationObserver !== 'undefined') {
        new MutationObserver(() => pointAtField(doc!, el, errorId)).observe(el, {
          characterData: true,
          childList: true,
          subtree: true,
        });
      }
      pointAtField(doc!, el, errorId);
    }
  }

  function scan(): void {
    repairPatterns(doc!);
    wrapPasswordFields(doc!);
    sweepOrphans(doc!);
    watchRefusals();
  }

  scan();

  if (typeof MutationObserver !== 'undefined' && doc.documentElement) {
    new MutationObserver(scan).observe(doc.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['pattern', 'type'],
    });
  }

  doc.addEventListener('invalid', (e) => markInvalid(e.target as Element | null), true);

  function onInput(e: Event): void {
    const target = e.target as HTMLInputElement | null;
    if (!target) return;
    if (typeof target.setCustomValidity === 'function') target.setCustomValidity('');
    if (target.validity && target.validity.valid) clearMark(target);
  }
  doc.addEventListener('input', onInput, true);
  doc.addEventListener('change', onInput, true);

  doc.addEventListener(
    'submit',
    (e) => {
      const form = e.target as HTMLFormElement | null;
      if (!form) return;
      if (typeof form.querySelectorAll === 'function') {
        const revealables = form.querySelectorAll<HTMLInputElement>(`[${REVEALABLE_ATTR}]`);
        concealAll(revealables);
        for (const field of revealables) {
          const button = field.nextElementSibling;
          if (button && button.classList.contains(REVEAL_CLASS)) syncReveal(button, false);
        }
      }
      if (form.noValidate && typeof form.checkValidity === 'function' && !form.checkValidity()) {
        e.preventDefault();
        e.stopImmediatePropagation();
        const first = firstInvalid(form.elements as unknown as ArrayLike<HTMLInputElement>);
        if (first) {
          markInvalid(first);
          if (typeof first.focus === 'function') first.focus();
        }
        if (typeof form.reportValidity === 'function') form.reportValidity();
      }
    },
    true,
  );
}
