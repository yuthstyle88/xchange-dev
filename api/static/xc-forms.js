// GENERATED from api/web/forms/src by api/web/forms/build.mjs — do not edit.
// Rebuild: npm --prefix api/web/forms ci && npm --prefix api/web/forms run build
"use strict";
(() => {
  // src/patterns.ts
  var LOGIN = "[a-zA-Z0-9._-]{3,30}";
  var LOGIN_FIXED = "[a-zA-Z0-9._\\-]{3,40}";
  var PATTERN_OVERRIDES = Object.freeze({
    [LOGIN]: LOGIN_FIXED
  });
  function backslashesBefore(s, i) {
    let count = 0;
    for (let j = i - 1; j >= 0 && s[j] === "\\"; j--) count++;
    return count;
  }
  function repairDashes(p) {
    let result = "";
    let inClass = false;
    for (let i = 0; i < p.length; i++) {
      const ch = p[i];
      const escaped = backslashesBefore(p, i) % 2 === 1;
      if (!inClass) {
        if (ch === "[" && !escaped) inClass = true;
      } else if (ch === "]" && !escaped) {
        if (result.endsWith("-") && backslashesBefore(result, result.length - 1) % 2 === 0) {
          result = result.slice(0, -1) + "\\-";
        }
        inClass = false;
      }
      result += ch;
    }
    return result;
  }
  function compiles(p) {
    try {
      new RegExp(`^(?:${p})$`, "v");
      return true;
    } catch {
      return false;
    }
  }
  function fixPattern(p) {
    if (typeof p !== "string") return null;
    if (Object.prototype.hasOwnProperty.call(PATTERN_OVERRIDES, p)) return PATTERN_OVERRIDES[p];
    if (compiles(p)) return null;
    const repaired = repairDashes(p);
    return repaired !== p && compiles(repaired) ? repaired : null;
  }

  // src/marks.ts
  function firstInvalid(elements) {
    if (!elements) return null;
    for (let i = 0; i < elements.length; i++) {
      const el = elements[i];
      if (el && el.willValidate && typeof el.checkValidity === "function" && !el.checkValidity()) {
        return el;
      }
    }
    return null;
  }
  function markInvalid(el) {
    if (!el) return;
    if (el.classList && typeof el.classList.add === "function") el.classList.add("xc-invalid");
    if (typeof el.setAttribute === "function") el.setAttribute("aria-invalid", "true");
  }
  function clearMark(el) {
    if (!el) return;
    if (el.classList && typeof el.classList.remove === "function") el.classList.remove("xc-invalid");
    if (typeof el.removeAttribute === "function") el.removeAttribute("aria-invalid");
  }

  // src/refusals.ts
  var BRANCH = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
  var PASSWORD = "รหัสผ่านต้องมี 12–128 ตัวอักษร";
  var REFUSALS = Object.freeze({
    "employee-error": Object.freeze({
      [BRANCH]: "employee-branch",
      [PASSWORD]: "employee-password"
    }),
    "reset-error": Object.freeze({
      [PASSWORD]: "reset-employee-password"
    })
  });
  function fieldFor(errorId, text) {
    if (typeof errorId !== "string" || typeof text !== "string") return null;
    if (!Object.prototype.hasOwnProperty.call(REFUSALS, errorId)) return null;
    const mapping = REFUSALS[errorId];
    const sentence = text.trim();
    return Object.prototype.hasOwnProperty.call(mapping, sentence) ? mapping[sentence] : null;
  }

  // src/password-field.ts
  var WRAPPER_CLASS = "xc-password";
  var REVEAL_CLASS = "xc-reveal";
  var REVEALABLE_ATTR = "data-xc-revealable";
  var REVEAL_ATTR = "data-xc-reveal";
  function revealLabel(shown) {
    return shown ? "ซ่อนรหัสผ่าน" : "แสดงรหัสผ่าน";
  }
  function flipReveal(input) {
    if (!input) return false;
    const start = input.selectionStart;
    const end = input.selectionEnd;
    const shown = input.type === "password";
    input.type = shown ? "text" : "password";
    if (typeof input.setSelectionRange === "function" && start != null && end != null) {
      try {
        input.setSelectionRange(start, end);
      } catch {
      }
    }
    if (typeof input.focus === "function") {
      try {
        input.focus();
      } catch {
      }
    }
    return shown;
  }
  function syncReveal(button, shown) {
    button.setAttribute("aria-label", revealLabel(shown));
    button.setAttribute("aria-pressed", String(shown));
  }
  var SVG_NS = "http://www.w3.org/2000/svg";
  function svg(doc, tag, attrs) {
    const el = doc.createElementNS(SVG_NS, tag);
    for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value);
    return el;
  }
  function makeRevealButton(doc, input) {
    const button = doc.createElement("button");
    button.type = "button";
    button.setAttribute("type", "button");
    button.classList.add(REVEAL_CLASS);
    syncReveal(button, false);
    const icon = svg(doc, "svg", {
      viewBox: "0 0 24 24",
      width: "18",
      height: "18",
      fill: "none",
      stroke: "currentColor",
      "stroke-width": "2",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true"
    });
    icon.appendChild(svg(doc, "path", { d: "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" }));
    icon.appendChild(svg(doc, "circle", { cx: "12", cy: "12", r: "3" }));
    button.appendChild(icon);
    button.addEventListener("click", (e) => {
      e.preventDefault();
      syncReveal(button, flipReveal(input));
    });
    return button;
  }
  function concealAll(inputs) {
    if (!inputs) return;
    for (let i = 0; i < inputs.length; i++) {
      const input = inputs[i];
      if (input && input.type === "text") input.type = "password";
    }
  }
  function isWrapper(node) {
    const el = node;
    return !!el && !!el.classList && typeof el.classList.contains === "function" && el.classList.contains(WRAPPER_CLASS);
  }
  function carryMargins(doc, input, wrapper) {
    const view = doc.defaultView;
    if (!view || typeof view.getComputedStyle !== "function") return;
    const style = view.getComputedStyle(input);
    wrapper.style.marginTop = style.marginTop;
    wrapper.style.marginBottom = style.marginBottom;
    input.style.marginTop = "0";
    input.style.marginBottom = "0";
  }
  function wrapPasswordField(doc, input) {
    const parent = input.parentNode;
    if (!parent) return null;
    if (isWrapper(parent)) return parent;
    const wrapper = doc.createElement("span");
    wrapper.classList.add(WRAPPER_CLASS);
    parent.insertBefore(wrapper, input);
    wrapper.appendChild(input);
    carryMargins(doc, input, wrapper);
    input.setAttribute(REVEALABLE_ATTR, "true");
    const button = makeRevealButton(doc, input);
    button.setAttribute(REVEAL_ATTR, "true");
    wrapper.appendChild(button);
    return wrapper;
  }

  // src/style.ts
  var STYLE_ID = "xc-forms-style";
  var BRAND = "var(--brand, #a5003c)";
  var INVALID = "#c62828";
  var EYE = "#6b6b66";
  var CSS = `
.xc-invalid, :user-invalid {
  border-color: ${INVALID} !important;
}
.xc-invalid:focus, :user-invalid:focus {
  outline: none;
}
.xc-password {
  display: block;
  position: relative;
}
.xc-password > input {
  display: block;
  width: 100%;
  box-sizing: border-box;
  padding-right: 44px;
}
.xc-password > input:focus {
  outline: none;
  border-color: ${BRAND};
}
.xc-password .xc-reveal {
  position: absolute;
  top: 0;
  bottom: 0;
  right: 0;
  width: 44px;
  margin: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: none;
  box-shadow: none;
  color: ${EYE};
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.xc-password .xc-reveal:hover,
.xc-password .xc-reveal[aria-pressed="true"] {
  color: ${BRAND};
}
.xc-password .xc-reveal:focus-visible {
  outline: 2px solid ${BRAND};
  outline-offset: -6px;
}
`;

  // src/install.ts
  function addStyle(doc) {
    if (doc.getElementById(STYLE_ID)) return;
    const style = doc.createElement("style");
    style.id = STYLE_ID;
    style.textContent = CSS;
    (doc.head || doc.documentElement)?.appendChild(style);
  }
  function repairPatterns(doc) {
    for (const el of doc.querySelectorAll("input[pattern]")) {
      const pattern = el.getAttribute("pattern");
      const fixed = fixPattern(pattern);
      if (fixed !== null && fixed !== pattern) el.setAttribute("pattern", fixed);
    }
  }
  function wrapPasswordFields(doc) {
    for (const el of doc.querySelectorAll('input[type="password"]')) {
      if (!el.hasAttribute(REVEALABLE_ATTR)) wrapPasswordField(doc, el);
    }
  }
  function sweepOrphans(doc) {
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
  function pointAtField(doc, errorEl, errorId) {
    const text = errorEl.textContent || "";
    const fieldId = fieldFor(errorId, text);
    if (!fieldId) return;
    const field = doc.getElementById(fieldId);
    if (!field) return;
    markInvalid(field);
    if (typeof field.setCustomValidity === "function") field.setCustomValidity(text.trim());
    if (typeof field.focus === "function") field.focus();
    if (typeof field.reportValidity === "function") field.reportValidity();
  }
  function install(doc) {
    if (!doc) return;
    addStyle(doc);
    const watchedErrors = /* @__PURE__ */ new Set();
    function watchRefusals() {
      for (const errorId of Object.keys(REFUSALS)) {
        const el = doc.getElementById(errorId);
        if (!el || watchedErrors.has(el)) continue;
        watchedErrors.add(el);
        if (typeof MutationObserver !== "undefined") {
          new MutationObserver(() => pointAtField(doc, el, errorId)).observe(el, {
            characterData: true,
            childList: true,
            subtree: true
          });
        }
        pointAtField(doc, el, errorId);
      }
    }
    function scan() {
      repairPatterns(doc);
      wrapPasswordFields(doc);
      sweepOrphans(doc);
      watchRefusals();
    }
    scan();
    if (typeof MutationObserver !== "undefined" && doc.documentElement) {
      new MutationObserver(scan).observe(doc.documentElement, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["pattern", "type"]
      });
    }
    doc.addEventListener("invalid", (e) => markInvalid(e.target), true);
    function onInput(e) {
      const target = e.target;
      if (!target) return;
      if (typeof target.setCustomValidity === "function") target.setCustomValidity("");
      if (target.validity && target.validity.valid) clearMark(target);
    }
    doc.addEventListener("input", onInput, true);
    doc.addEventListener("change", onInput, true);
    doc.addEventListener(
      "submit",
      (e) => {
        const form = e.target;
        if (!form) return;
        if (typeof form.querySelectorAll === "function") {
          const revealables = form.querySelectorAll(`[${REVEALABLE_ATTR}]`);
          concealAll(revealables);
          for (const field of revealables) {
            const button = field.nextElementSibling;
            if (button && button.classList.contains(REVEAL_CLASS)) syncReveal(button, false);
          }
        }
        if (form.noValidate && typeof form.checkValidity === "function" && !form.checkValidity()) {
          e.preventDefault();
          e.stopImmediatePropagation();
          const first = firstInvalid(form.elements);
          if (first) {
            markInvalid(first);
            if (typeof first.focus === "function") first.focus();
          }
          if (typeof form.reportValidity === "function") form.reportValidity();
        }
      },
      true
    );
  }

  // src/index.ts
  if (typeof module === "object" && module && module.exports) {
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
      wrapPasswordField
    };
  } else if (typeof document !== "undefined") {
    install(document);
  }
})();
