// S-9 #1223 (§7 `home.ts` [required], §8): `safeNext` decides whether the sign-in screen may send a person to the
// `next` it was given. Design docs/design/xchange-v3-wave3-staff.md row S-9 (:216, :218) and its test row (:229):
// the value is URL-decoded once, then accepted only if it starts with exactly one `/` and its second character is
// neither `/` nor `\`; anything else is refused, so the sign-in screen is not an open redirect.
// Tightened by the lane controller's ruling on #1223 (issuecomment-6103413078): after the one decode, the value must
// also hold no control character (U+0000–U+001F, U+007F) and no `\` anywhere, keep the page's origin, and not be
// `/login` (bare or followed by `?`, `#` or `/`) or anything under `/api/`.
// Rule 6 (ruling issuecomment-6103468024): the value must already be what the browser would navigate to, i.e.
// new URL(v, origin)'s pathname + search + hash equals v byte for byte, so no dot segment can resolve to /login or /api/.
// Written before the implementation by the test writer (claude/5d45b017). Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/home.ts`:
//
//   export function safeNext(raw: string | null | undefined): string | null;
//
// It is read through import.meta.glob so a missing export fails these tests, not the typecheck.
import { describe, expect, it } from 'vitest';

type SafeNext = (raw: string | null | undefined) => string | null;

const modules = import.meta.glob<{ safeNext?: unknown }>('./home.ts', { eager: true });

function safeNext(raw: string | null | undefined): string | null {
  const fn = modules['./home.ts']?.safeNext;
  expect(fn, 'web/src/screens/login/home.ts exports safeNext').toBeTypeOf('function');
  return (fn as SafeNext)(raw);
}

describe('safeNext: accepted values come back as the path, decoded once (§8)', () => {
  it.each([
    ['%2Fstaff', '/staff'],
    ['/staff', '/staff'],
    ['%2Fstaff%3Ftab%3Drates', '/staff?tab=rates'],
    ['/desk', '/desk'],
    ['%2Fstaff%3Ftab%3Dfloor', '/staff?tab=floor'],
    // one decode only: %2525 becomes %25 and stays %25 (no second decode, no re-encode)
    ['%2Fstaff%3Fq%3D%2525', '/staff?q=%25'],
  ])('%s → %s', (raw, expected) => {
    expect(safeNext(raw)).toBe(expected);
  });
});

describe('safeNext: refused values answer null (§8, SD:229)', () => {
  it.each([
    ['//evil', 'protocol-relative'],
    ['/\\evil.com', 'backslash, which browsers read as a slash'],
    ['%2F%2Fevil', 'protocol-relative once decoded'],
    ['/%2Fevil', 'protocol-relative once decoded, though the raw value starts with one slash'],
    ['/%5Cevil.com', 'backslash once decoded'],
    ['https://evil', 'absolute URL'],
    ['https%3A%2F%2Fevil', 'absolute URL once decoded'],
    ['javascript:alert(1)', 'script URL'],
    ['javascript%3Aalert(1)', 'script URL once decoded'],
    ['staff', 'no leading slash'],
    [' /staff', 'leading space'],
    ['', 'empty'],
    // ruling issuecomment-6103413078: a control character that browsers strip turns /x/ into //
    ['%2F%09%2Fevil.com', 'tab between the slashes once decoded'],
    ['%2F%0A%2Fevil.com', 'line feed between the slashes once decoded'],
    ['%2F%0D%2Fevil.com', 'carriage return between the slashes once decoded'],
    ['%2Fstaff%00', 'NUL once decoded'],
    ['%2Fstaff%7F', 'DEL once decoded'],
    ['%2Fstaff%5Cx', 'a backslash anywhere once decoded'],
    // ruling: never the sign-in screen itself, never an API route
    ['%2Flogin', 'the sign-in screen'],
    ['%2Flogin%3Fnext%3D%252Fstaff', 'the sign-in screen with its own next'],
    ['%2Flogin%23top', 'the sign-in screen with a fragment'],
    ['%2Flogin%2F', 'the sign-in screen with a trailing slash'],
    ['%2Fapi%2Fv3%2Fviews%2Fstaff_me', 'an API route'],
    // rule 6: dot segments the browser resolves away (issuecomment-6103468024)
    ['%2Fstaff%2F..%2Flogin', 'resolves to /login'],
    ['%2Fstaff%2F%252e%252e%2Flogin', 'encoded dot segment, resolves to /login'],
    ['%2Fstaff%2F%252E%252E%2Flogin', 'upper-case encoded dot segment, resolves to /login'],
    ['%2Fx%2F..%2Fapi%2Fv3%2Fviews%2Fstaff_me', 'resolves to an API route'],
    ['%2Fstaff%2F.%2Ffloor', 'a single dot segment: not what the browser would navigate to'],
    ['%2Fstaff%3Fq%3Da%20b', 'a query the browser would re-encode (space → %20): not byte for byte'],
    ['%2Fstaff%23a%20b', 'a fragment the browser would re-encode (space → %20): not byte for byte'],
  ])('%j (%s) → null', (raw) => {
    expect(safeNext(raw)).toBeNull();
  });

  it('answers null for an absent value', () => {
    expect(safeNext(null)).toBeNull();
    expect(safeNext(undefined)).toBeNull();
  });

  it('decodes exactly once: a twice-encoded slash is not a path after one decode', () => {
    // one decode of %252Fstaff is the text %2Fstaff, which does not start with "/"
    expect(safeNext('%252Fstaff')).toBeNull();
    // and %252F%252Fevil must not become //evil, nor /evil
    expect(safeNext('%252F%252Fevil')).toBeNull();
  });

  it('answers null, and does not throw, for a value that is not valid percent-encoding', () => {
    expect(() => safeNext('%E0%A4%A')).not.toThrow();
    expect(safeNext('%E0%A4%A')).toBeNull();
    expect(safeNext('/staff%')).toBeNull();
  });
});
