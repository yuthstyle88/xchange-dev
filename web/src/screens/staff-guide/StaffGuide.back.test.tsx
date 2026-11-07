// OL-6 guide-back: the owner's answer 3A on #564 (issuecomment-6705475792, 2026-11-07, on the question in
// issuecomment-6705285102), shots 014 and 015. Every role saw "กลับแอดมิน" → /, though a teller has no admin work.
// Now the link depends on the caller:
// - admins and managers: "กลับแอดมิน" → `/`;
// - everyone else, and anyone whose `me` is not known (yet): "กลับหน้าพนักงาน" → `/staff`.
// "Admins and managers" is read from `me.permissions`, from the same shared session AppShell already reads
// (`shared/ui/AppShell.tsx:25,44`), never from a role name: the page `/` itself opens for any signed-in caller with a
// branch (`api/src/presentation/pages.rs:166-171`, `PageKind::Branch`, no permission), so the screen uses the
// permission the admin cash page checks for shift-close certification, `ShiftCloseCertify` (cash/CashFlows.tsx VERIFY),
// which the built-in groups give exactly to Executive, Admin and BranchManager and not to Teller, Finance or
// RateManager (`api/crates/xc-auth/src/domain/access/role_grants.rs:229-327`). Measured on xc-v3 17:3x ICT 2026-11-07:
// owner (ADMIN+EXECUTIVE), admin, executive hold it; teller does not.
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import { makeMe } from '../../shared/testing/fixtures';
import { en, th } from './i18n';
import { StaffGuide } from './StaffGuide';

let dispose: (() => void) | undefined;

beforeEach(() => {
  window.localStorage.clear();
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function signIn(permissions: string[]): void {
  sharedSession().updateMe(makeMe({ id: 'u-1', name: 'ทดสอบ', roles: [], permissions, branch: 'MAIN' }));
}

function back(): HTMLAnchorElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <StaffGuide print={() => undefined} />, host);
  const link = host.querySelector<HTMLAnchorElement>('nav a');
  expect(link, 'the back link in the nav line').not.toBeNull();
  return link!;
}

describe('guide-back 3A: the back link follows the caller', () => {
  it('GB-1 a holder of ShiftCloseCertify (admin, executive, branch manager) sees กลับแอดมิน → /', () => {
    signIn(['SignedIn', 'CompatRead', 'ShiftCloseCertify']);
    const a = back();
    expect(a.textContent).toBe('กลับแอดมิน');
    expect(a.getAttribute('href')).toBe('/');
  });

  it('GB-2 a teller (CompatRead, ShiftOperate, no ShiftCloseCertify) sees กลับหน้าพนักงาน → /staff', () => {
    signIn(['SignedIn', 'CompatRead', 'ShiftOperate', 'DocumentsList']);
    const a = back();
    expect(a.textContent).toBe('กลับหน้าพนักงาน');
    expect(a.getAttribute('href')).toBe('/staff');
  });

  it('GB-3 finance (CashManage, ApprovalsDecide, no ShiftCloseCertify) also goes back to /staff', () => {
    signIn(['SignedIn', 'CompatRead', 'CashManage', 'ApprovalsDecide']);
    const a = back();
    expect(a.getAttribute('href')).toBe('/staff');
  });

  it('GB-4 in English: Back to Admin / Back to the staff page', () => {
    setLang('en');
    signIn(['ShiftCloseCertify']);
    expect(back().textContent).toBe(en.back);
    dispose?.();
    document.body.innerHTML = '';
    signIn([]);
    expect(back().textContent).toBe('Back to the staff page');
  });

  it('GB-5 the key is backStaff in both tables; back keeps v2 words', () => {
    expect((th as Record<string, string>).backStaff).toBe('กลับหน้าพนักงาน');
    expect((en as Record<string, string>).backStaff).toBe('Back to the staff page');
    expect(th.back).toBe('กลับแอดมิน');
  });

  it('GB-6 the link follows a change of me without a reload', () => {
    signIn([]);
    const a = back();
    expect(a.getAttribute('href')).toBe('/staff');
    signIn(['ShiftCloseCertify']);
    expect(a.getAttribute('href')).toBe('/');
    expect(a.textContent).toBe('กลับแอดมิน');
  });
});
