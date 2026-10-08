// V1-login (#396 §7, `home.ts` [required]): where a signed-in user goes from `/login`.
// v2: `const home=u=>u.role==='FRONT'?'/desk':'/'`
// (xchange-v2-source@4d0bdbb:src/devices-public/login.html:2).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/home.ts`:
//
//   export function homeOf(user: { role: string }): string;
import { describe, expect, it } from 'vitest';
import { homeOf } from './home';

describe('homeOf (#396)', () => {
  it('sends FRONT to /desk', () => {
    expect(homeOf({ role: 'FRONT' })).toBe('/desk');
  });

  it('sends the owner to /', () => {
    expect(homeOf({ role: 'OWNER' })).toBe('/');
  });

  it('sends every other staff role to /', () => {
    for (const role of ['RATES', 'BACKEND', 'MANAGER', '']) {
      expect(homeOf({ role })).toBe('/');
    }
  });

  it('compares the role exactly, as v2 does with ===', () => {
    expect(homeOf({ role: 'front' })).toBe('/');
    expect(homeOf({ role: ' FRONT' })).toBe('/');
  });
});
