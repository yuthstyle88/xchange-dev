// The helper's one style sheet, added once per page as <style id="xc-forms-style">.
//
// The look the owner chose (#378): one border, the eye inside the field on the right.
// An invalid field is marked by a red border only; a focused field changes its border colour
// instead of drawing the page's outer focus ring around it.

export const STYLE_ID = 'xc-forms-style';

// The page's own brand colour when it defines one (the kept admin page: --brand #a5003c).
const BRAND = 'var(--brand, #a5003c)';
const INVALID = '#c62828';
const EYE = '#6b6b66';

export const CSS = `
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
