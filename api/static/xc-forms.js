(function () {
  'use strict';

  const LOGIN = '[a-zA-Z0-9._-]{3,30}';
  const LOGIN_FIXED = '[a-zA-Z0-9._\\-]{3,40}';

  const PATTERN_OVERRIDES = Object.freeze({
    [LOGIN]: LOGIN_FIXED,
  });

  const BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
  const PASSWORD = 'รหัสผ่านต้องมี 12–128 ตัวอักษร'; // U+2013 en-dash

  const REFUSALS = Object.freeze({
    'employee-error': Object.freeze({
      [BRANCH]: 'employee-branch',
      [PASSWORD]: 'employee-password',
    }),
    'reset-error': Object.freeze({
      [PASSWORD]: 'reset-employee-password',
    }),
  });

  const CSS = `
.xc-invalid, :user-invalid {
  border-color: #c62828 !important;
  background-color: #fdecea !important;
}
.xc-reveal {
  background: none;
  border: none;
  cursor: pointer;
  padding: 0 4px;
  vertical-align: middle;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
`;

  function repairDashes(p) {
    let result = '';
    let inClass = false;
    let i = 0;
    while (i < p.length) {
      if (!inClass) {
        if (p[i] === '[') {
          let bs = 0;
          let j = i - 1;
          while (j >= 0 && p[j] === '\\') {
            bs++;
            j--;
          }
          if (bs % 2 === 0) {
            inClass = true;
          }
        }
        result += p[i];
        i++;
      } else {
        if (p[i] === ']') {
          let bs = 0;
          let j = i - 1;
          while (j >= 0 && p[j] === '\\') {
            bs++;
            j--;
          }
          if (bs % 2 === 0) {
            if (result.endsWith('-')) {
              let dashBs = 0;
              let k = result.length - 2;
              while (k >= 0 && result[k] === '\\') {
                dashBs++;
                k--;
              }
              if (dashBs % 2 === 0) {
                result = result.slice(0, result.length - 1) + '\\-';
              }
            }
            inClass = false;
          }
        }
        result += p[i];
        i++;
      }
    }
    return result;
  }

  function fixPattern(p) {
    if (typeof p !== 'string') return null;
    if (Object.prototype.hasOwnProperty.call(PATTERN_OVERRIDES, p)) {
      return PATTERN_OVERRIDES[p];
    }
    try {
      new RegExp(`^(?:${p})$`, 'v');
      return null;
    } catch (_) {}

    const repaired = repairDashes(p);
    if (repaired !== p) {
      try {
        new RegExp(`^(?:${repaired})$`, 'v');
        return repaired;
      } catch (_) {}
    }
    return null;
  }

  function firstInvalid(elements) {
    if (!elements) return null;
    for (let i = 0; i < elements.length; i++) {
      const el = elements[i];
      if (el && el.willValidate && typeof el.checkValidity === 'function' && !el.checkValidity()) {
        return el;
      }
    }
    return null;
  }

  function markInvalid(el) {
    if (!el) return;
    if (el.classList && typeof el.classList.add === 'function') {
      el.classList.add('xc-invalid');
    }
    if (typeof el.setAttribute === 'function') {
      el.setAttribute('aria-invalid', 'true');
    }
  }

  function clearMark(el) {
    if (!el) return;
    if (el.classList && typeof el.classList.remove === 'function') {
      el.classList.remove('xc-invalid');
    }
    if (typeof el.removeAttribute === 'function') {
      el.removeAttribute('aria-invalid');
    }
  }

  function fieldFor(errorId, text) {
    if (typeof errorId !== 'string' || typeof text !== 'string') return null;
    if (!Object.prototype.hasOwnProperty.call(REFUSALS, errorId)) return null;
    const mapping = REFUSALS[errorId];
    const trimmed = text.trim();
    if (Object.prototype.hasOwnProperty.call(mapping, trimmed)) {
      return mapping[trimmed];
    }
    return null;
  }

  function revealLabel(shown) {
    return shown ? 'ซ่อนรหัสผ่าน' : 'แสดงรหัสผ่าน';
  }

  function flipReveal(input) {
    if (!input) return false;
    const start = input.selectionStart;
    const end = input.selectionEnd;
    const shown = input.type === 'password';
    input.type = shown ? 'text' : 'password';
    if (
      typeof input.setSelectionRange === 'function' &&
      start !== null &&
      end !== null &&
      start !== undefined &&
      end !== undefined
    ) {
      try {
        input.setSelectionRange(start, end);
      } catch (_) {}
    }
    if (typeof input.focus === 'function') {
      try {
        input.focus();
      } catch (_) {}
    }
    return shown;
  }

  function makeRevealButton(doc, input) {
    const btn = doc.createElement('button');
    btn.type = 'button';
    btn.setAttribute('type', 'button');
    btn.classList.add('xc-reveal');
    btn.setAttribute('aria-label', revealLabel(false));
    btn.setAttribute('aria-pressed', 'false');

    const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('width', '18');
    svg.setAttribute('height', '18');
    svg.setAttribute('fill', 'none');
    svg.setAttribute('stroke', 'currentColor');
    svg.setAttribute('stroke-width', '2');
    svg.setAttribute('stroke-linecap', 'round');
    svg.setAttribute('stroke-linejoin', 'round');
    svg.setAttribute('aria-hidden', 'true');

    const path = doc.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', 'M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z');
    svg.appendChild(path);

    const circle = doc.createElementNS('http://www.w3.org/2000/svg', 'circle');
    circle.setAttribute('cx', '12');
    circle.setAttribute('cy', '12');
    circle.setAttribute('r', '3');
    svg.appendChild(circle);

    btn.appendChild(svg);

    btn.addEventListener('click', function (e) {
      e.preventDefault();
      const shown = flipReveal(input);
      btn.setAttribute('aria-label', revealLabel(shown));
      btn.setAttribute('aria-pressed', String(shown));
    });

    return btn;
  }

  function concealAll(inputs) {
    if (!inputs) return;
    for (let i = 0; i < inputs.length; i++) {
      const input = inputs[i];
      if (input && input.type === 'text') {
        input.type = 'password';
      }
    }
  }

  function install(doc) {
    if (!doc) return;

    if (!doc.getElementById('xc-forms-style')) {
      const style = doc.createElement('style');
      style.id = 'xc-forms-style';
      style.textContent = CSS;
      const target = doc.head || doc.documentElement;
      if (target) {
        target.appendChild(style);
      }
    }

    function scan() {
      const patternInputs = doc.querySelectorAll('input[pattern]');
      for (let i = 0; i < patternInputs.length; i++) {
        const el = patternInputs[i];
        const p = el.getAttribute('pattern');
        const fixed = fixPattern(p);
        if (fixed !== null && fixed !== p) {
          el.setAttribute('pattern', fixed);
        }
      }

      const passwordInputs = doc.querySelectorAll('input[type="password"]');
      for (let i = 0; i < passwordInputs.length; i++) {
        const el = passwordInputs[i];
        if (!el.hasAttribute('data-xc-revealable')) {
          el.setAttribute('data-xc-revealable', 'true');
          const btn = makeRevealButton(doc, el);
          btn.setAttribute('data-xc-reveal', 'true');
          el.insertAdjacentElement('afterend', btn);
        }
      }

      const revealButtons = doc.querySelectorAll('[data-xc-reveal]');
      for (let i = 0; i < revealButtons.length; i++) {
        const btn = revealButtons[i];
        const prev = btn.previousElementSibling;
        if (!prev || !doc.contains(prev) || !prev.hasAttribute('data-xc-revealable')) {
          btn.remove();
        }
      }

      watchRefusals();
    }

    const watchedErrors = new Set();
    function watchRefusals() {
      for (const errorId of Object.keys(REFUSALS)) {
        const el = doc.getElementById(errorId);
        if (el && !watchedErrors.has(el)) {
          watchedErrors.add(el);
          if (typeof MutationObserver !== 'undefined') {
            const obs = new MutationObserver(function () {
              handleRefusal(el, errorId);
            });
            obs.observe(el, { characterData: true, childList: true, subtree: true });
          }
          handleRefusal(el, errorId);
        }
      }
    }

    function handleRefusal(el, errorId) {
      const text = el.textContent || '';
      const fieldId = fieldFor(errorId, text);
      if (fieldId) {
        const field = doc.getElementById(fieldId);
        if (field) {
          markInvalid(field);
          if (typeof field.setCustomValidity === 'function') {
            field.setCustomValidity(text.trim());
          }
          if (typeof field.focus === 'function') {
            field.focus();
          }
          if (typeof field.reportValidity === 'function') {
            field.reportValidity();
          }
        }
      }
    }

    scan();

    if (typeof MutationObserver !== 'undefined' && doc.documentElement) {
      const observer = new MutationObserver(function () {
        scan();
      });
      observer.observe(doc.documentElement, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ['pattern', 'type'],
      });
    }

    doc.addEventListener(
      'invalid',
      function (e) {
        markInvalid(e.target);
      },
      true,
    );

    function onInput(e) {
      const target = e.target;
      if (!target) return;
      if (typeof target.setCustomValidity === 'function') {
        target.setCustomValidity('');
      }
      if (target.validity && target.validity.valid) {
        clearMark(target);
      }
    }
    doc.addEventListener('input', onInput, true);
    doc.addEventListener('change', onInput, true);

    doc.addEventListener(
      'submit',
      function (e) {
        const form = e.target;
        if (!form) return;
        if (typeof form.querySelectorAll === 'function') {
          const revealables = form.querySelectorAll('[data-xc-revealable]');
          concealAll(revealables);
          for (let i = 0; i < revealables.length; i++) {
            const btn = revealables[i].nextElementSibling;
            if (btn && btn.classList && btn.classList.contains('xc-reveal')) {
              btn.setAttribute('aria-label', revealLabel(false));
              btn.setAttribute('aria-pressed', 'false');
            }
          }
        }
        if (form.noValidate && typeof form.checkValidity === 'function' && !form.checkValidity()) {
          e.preventDefault();
          e.stopImmediatePropagation();
          const first = firstInvalid(form.elements);
          if (first) {
            markInvalid(first);
            if (typeof first.focus === 'function') {
              first.focus();
            }
          }
          if (typeof form.reportValidity === 'function') {
            form.reportValidity();
          }
        }
      },
      true,
    );
  }

  if (typeof module === 'object' && module.exports) {
    module.exports = {
      fixPattern,
      PATTERN_OVERRIDES,
      firstInvalid,
      markInvalid,
      clearMark,
      CSS,
      REFUSALS,
      fieldFor,
      revealLabel,
      flipReveal,
      makeRevealButton,
      concealAll,
    };
  } else if (typeof document !== 'undefined') {
    install(document);
  }
})();
