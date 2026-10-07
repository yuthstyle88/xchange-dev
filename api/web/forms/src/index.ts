// Entry point of api/static/xc-forms.js, a classic browser script served at /_xc/forms.js.
//
// In a browser it installs itself on `document`. Under Node, where `module` exists, it
// installs nothing and exports its pure parts for api/tests/js/*.test.mjs.

import { PATTERN_OVERRIDES, fixPattern } from './patterns';
import { clearMark, firstInvalid, markInvalid } from './marks';
import { REFUSALS, fieldFor } from './refusals';
import { concealAll, flipReveal, makeRevealButton, revealLabel, wrapPasswordField } from './password-field';
import { CSS } from './style';
import { install } from './install';

declare const module: { exports: unknown } | undefined;

if (typeof module === 'object' && module && module.exports) {
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
    wrapPasswordField,
  };
} else if (typeof document !== 'undefined') {
  install(document);
}
