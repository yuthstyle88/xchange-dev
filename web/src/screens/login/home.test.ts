// OR-3 web follow-up (#762 §7 `home.ts` [required]): where a signed-in user goes from `/login`,
// now that the BFF answers `roles: [{role, branchId}]` in Blueprint names
// (api/crates/xc-auth/src/application/session/session_service.rs:27-41 @ 8234e77) instead of one v2 `role`.
// The rule mirrors the server's own bridge to the v2 actor, so `/login` sends a person where the pages will
// treat them the same way (api/src/presentation/extractors/actor.rs:142-185 @ 8234e77):
//   EXECUTIVE and ADMIN both organisation-wide (branchId null) → the owner → `/`;
//   otherwise any TELLER role (v2 FRONT, first in the priority list) → `/desk`;
//   everyone else → `/`.
// Replaces the #396 pins of `homeOf({role})` (the retired file's :12-30: FRONT → /desk).
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/home.ts`:
//
//   export function homeOf(user: { roles: ReadonlyArray<{ role: string; branchId: string | null }> }): string;
import { describe, expect, it } from 'vitest';

type Role = { role: string; branchId: string | null };
type HomeOf = (user: { roles: ReadonlyArray<Role> }) => string;

const modules = import.meta.glob<{ homeOf?: unknown }>('./home.ts', { eager: true });

function homeOf(user: { roles: ReadonlyArray<Role> }): string {
  const fn = modules['./home.ts']?.homeOf;
  expect(fn, 'web/src/screens/login/home.ts exports homeOf').toBeTypeOf('function');
  return (fn as HomeOf)(user);
}

describe('homeOf over Blueprint roles (OR-3 web)', () => {
  it('sends a TELLER to /desk', () => {
    expect(homeOf({ roles: [{ role: 'TELLER', branchId: 'XC-B01' }] })).toBe('/desk');
  });

  it('sends a TELLER who also holds another role to /desk, whatever the order', () => {
    expect(
      homeOf({
        roles: [
          { role: 'RATE_MANAGER', branchId: 'XC-B01' },
          { role: 'TELLER', branchId: 'XC-B02' },
        ],
      }),
    ).toBe('/desk');
  });

  it('sends the owner (EXECUTIVE + ADMIN, both organisation-wide) to /, even holding TELLER', () => {
    expect(
      homeOf({
        roles: [
          { role: 'EXECUTIVE', branchId: null },
          { role: 'ADMIN', branchId: null },
        ],
      }),
    ).toBe('/');
    expect(
      homeOf({
        roles: [
          { role: 'TELLER', branchId: 'XC-B01' },
          { role: 'EXECUTIVE', branchId: null },
          { role: 'ADMIN', branchId: null },
        ],
      }),
    ).toBe('/');
  });

  it('is not the owner when ADMIN or EXECUTIVE is scoped to a branch: a TELLER then goes to /desk', () => {
    expect(
      homeOf({
        roles: [
          { role: 'EXECUTIVE', branchId: null },
          { role: 'ADMIN', branchId: 'XC-B01' },
          { role: 'TELLER', branchId: 'XC-B01' },
        ],
      }),
    ).toBe('/desk');
    expect(
      homeOf({
        roles: [
          { role: 'EXECUTIVE', branchId: 'XC-B01' },
          { role: 'ADMIN', branchId: null },
          { role: 'TELLER', branchId: 'XC-B01' },
        ],
      }),
    ).toBe('/desk');
  });

  it('sends every other role, and no role, to /', () => {
    for (const role of ['BRANCH_MANAGER', 'RATE_MANAGER', 'FINANCE', 'ADMIN', 'VERIFICATION_REVIEWER', 'EXECUTIVE']) {
      expect(homeOf({ roles: [{ role, branchId: 'XC-B01' }] }), role).toBe('/');
    }
    expect(homeOf({ roles: [] })).toBe('/');
  });

  it('compares the role exactly: v2 names and other spellings are not TELLER', () => {
    for (const role of ['FRONT', 'teller', ' TELLER', 'TELLER ']) {
      expect(homeOf({ roles: [{ role, branchId: 'XC-B01' }] }), JSON.stringify(role)).toBe('/');
    }
  });
});
