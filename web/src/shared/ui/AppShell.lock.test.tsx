// V0-B-session-end (#492; design docs/design/xchange-v3-web.md §12.19.5, D-S4): when the shared
// session has ended, AppShell says so and locks the page's controls, as v2's account bar did
// (xchange-v2-source@4d0bdbb:src/devices-public/account-bar.js:42 lock, :44 the two sentences).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// What this test pins, in `web/src/shared/ui/AppShell.tsx`:
//
//   - While `sharedSession().ended()` is set, one `<p role="alert">` after the `<header>` and before
//     the content shows t('shell.session.expired') or t('shell.session.switched').
//   - The content wrapper (`<main>`, or `<div class="xc-content">` with `ownsMain`) sits inside a
//     `<fieldset class="xc-lock">`, always rendered, disabled exactly while the session has ended.
//     Every button, input, select and textarea of the content is then :disabled, in a form or not.
//   - The `<header>` (LangSwitch) and the account bar (AccountBar's sign-out) stay outside the
//     fieldset and usable.
//
// Amended 2026-10-10 by claude/c954feda on the owner's order (xchange#917 follow-up): the account
// bar moved from inside `<header>` to just before it, where v2 put it (account-bar.js:12
// `document.body.prepend(bar)`). Every check is kept; the sign-out is looked up in the bar instead
// of the header, and the bar is also checked to be outside the fieldset and before the header.
//
// Texts: two shared keys under `shell.*` (W10) in web/src/shared/i18n/th.ts and en.ts. The Thai is
// v2's, verbatim (account-bar.js:44). The English is the implementer's ([proposed] in the design).
//
// The shared session is a page singleton, so every test loads fresh modules. Nothing here is JSX:
// the test builds the shell with the freshly loaded Solid, so both sides share one Solid instance.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import type { Me } from '../api/generated/common';

const EXPIRED_TH = 'เซสชันสิ้นสุด กรุณาเข้าสู่ระบบใหม่';
const SWITCHED_TH = 'บัญชีเปลี่ยนในหน้าต่างอื่น กรุณาเข้าสู่ระบบใหม่ก่อนทำรายการ';
const THAI = /[฀-๿]/;

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};
const OWNER: Me = { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Somchai', username: 'owner1' };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** /health/ready, the me view as the owner, and 401 for any other view read. */
function stubNetwork() {
  const fetch = vi.fn(async (input: string) => {
    const url = String(input);
    if (url.startsWith('/health/ready')) return json(200, READY);
    if (url.startsWith('/api/v3/views/me')) return json(200, { version: 3, changed: true, data: OWNER });
    return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

let cleanup: Array<() => void> = [];

async function load() {
  vi.resetModules();
  const web = await import('solid-js/web');
  const solid = await import('solid-js');
  const { AppShell } = await import('./AppShell');
  const session = await import('../session/session');
  const i18n = await import('../i18n');
  const { th } = await import('../i18n/th');
  const { en } = await import('../i18n/en');
  return { web, solid, AppShell, session, i18n, th, en };
}

type Loaded = Awaited<ReturnType<typeof load>>;

/** Controls in a form and outside one, as a screen renders them. */
function controls(): HTMLElement {
  const box = document.createElement('section');
  box.innerHTML = `
    <form id="f">
      <input name="amount" />
      <select name="ccy"><option>USD</option></select>
      <textarea name="note"></textarea>
      <button type="submit" id="save">save</button>
    </form>
    <button type="button" id="loose-button">loose</button>
    <input id="loose-input" />
    <select id="loose-select"><option>a</option></select>
    <textarea id="loose-textarea"></textarea>
    <a href="#x" id="link">link</a>
  `;
  return box;
}

function mount(m: Loaded, ownsMain: boolean): HTMLElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const children = (): JSX.Element => {
    if (!ownsMain) return controls();
    const main = document.createElement('main');
    main.className = 'child-main';
    main.appendChild(controls());
    return main;
  };
  const dispose = m.web.render(
    () =>
      m.solid.createComponent(m.AppShell, {
        title: 'แลกเงิน',
        ownsMain,
        get children() {
          return children();
        },
      }),
    host,
  );
  cleanup.push(dispose, () => m.session.sharedSession().dispose());
  return host;
}

const CONTROLS = 'button, input, select, textarea';

function contentControls(host: HTMLElement): HTMLElement[] {
  const fieldset = host.querySelector('fieldset.xc-lock');
  return Array.from(fieldset?.querySelectorAll<HTMLElement>(CONTROLS) ?? []);
}

async function signedIn(host: HTMLElement) {
  await vi.waitFor(() => {
    expect(host.querySelector('.account-bar')).not.toBeNull();
  });
}

beforeEach(() => {
  try {
    window.localStorage.clear();
  } catch {
    // storage blocked: the language starts as th anyway
  }
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(async () => {
  for (const f of cleanup) f();
  cleanup = [];
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

for (const ownsMain of [false, true]) {
  const wrapper = ownsMain ? 'div.xc-content' : 'main';
  describe(`AppShell lock, ${ownsMain ? 'with' : 'without'} ownsMain`, () => {
    it(`while the session runs: the ${wrapper} sits in an enabled fieldset.xc-lock, no notice, header outside`, async () => {
      stubNetwork();
      const m = await load();
      const host = mount(m, ownsMain);
      await signedIn(host);

      const fieldsets = host.querySelectorAll('fieldset.xc-lock');
      expect(fieldsets).toHaveLength(1);
      const fieldset = fieldsets[0] as HTMLFieldSetElement;
      expect(fieldset.disabled).toBe(false);
      expect(fieldset.querySelector(ownsMain ? ':scope > div.xc-content' : ':scope > main')).not.toBeNull();
      expect(host.querySelectorAll('main')).toHaveLength(1);
      if (ownsMain) expect(fieldset.querySelector('div.xc-content > main.child-main')).not.toBeNull();
      else expect(host.querySelector('.xc-content')).toBeNull();

      const header = host.querySelector('header')!;
      expect(header.closest('fieldset')).toBeNull();
      expect(fieldset.contains(header)).toBe(false);

      expect(host.querySelector('[role="alert"]')).toBeNull();
      const all = contentControls(host);
      expect(all).toHaveLength(8);
      for (const el of all) expect(el.matches(':disabled'), el.outerHTML).toBe(false);
    });

    it('end(expired): the Thai notice between header and content, every content control disabled', async () => {
      stubNetwork();
      const m = await load();
      const host = mount(m, ownsMain);
      await signedIn(host);
      const before = contentControls(host);

      m.session.sharedSession().end('expired');

      await vi.waitFor(() => {
        expect(host.querySelectorAll('[role="alert"]')).toHaveLength(1);
      });
      const alert = host.querySelector('[role="alert"]')!;
      expect(alert.tagName).toBe('P');
      expect(alert.textContent).toBe(EXPIRED_TH);
      expect(m.th['shell.session.expired']).toBe(EXPIRED_TH);

      const header = host.querySelector('header')!;
      const content = host.querySelector(wrapper)!;
      expect(header.contains(alert)).toBe(false);
      expect(header.compareDocumentPosition(alert) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(alert.compareDocumentPosition(content) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

      const fieldset = host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement;
      expect(fieldset.disabled).toBe(true);
      const after = contentControls(host);
      expect(after).toHaveLength(8);
      after.forEach((el, i) => expect(el).toBe(before[i]));
      for (const el of after) expect(el.matches(':disabled'), el.outerHTML).toBe(true);
    });

    it('a click on the locked submit button submits nothing; a locked loose button gets no click', async () => {
      stubNetwork();
      const m = await load();
      const host = mount(m, ownsMain);
      await signedIn(host);
      const form = host.querySelector('form#f') as HTMLFormElement;
      const submit = vi.fn((e: Event) => e.preventDefault());
      form.addEventListener('submit', submit);
      const loose = host.querySelector('#loose-button') as HTMLButtonElement;
      const clicked = vi.fn();
      loose.addEventListener('click', clicked);

      (host.querySelector('#save') as HTMLButtonElement).click();
      loose.click();
      expect(submit).toHaveBeenCalledTimes(1);
      expect(clicked).toHaveBeenCalledTimes(1);

      m.session.sharedSession().end('expired');
      await vi.waitFor(() => {
        expect((host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement).disabled).toBe(true);
      });
      (host.querySelector('#save') as HTMLButtonElement).click();
      loose.click();
      expect(submit).toHaveBeenCalledTimes(1);
      expect(clicked).toHaveBeenCalledTimes(1);
    });

    it('end(switched): the Thai switched sentence', async () => {
      stubNetwork();
      const m = await load();
      const host = mount(m, ownsMain);
      await signedIn(host);
      m.session.sharedSession().end('switched');
      await vi.waitFor(() => {
        expect(host.querySelector('[role="alert"]')?.textContent).toBe(SWITCHED_TH);
      });
      expect(m.th['shell.session.switched']).toBe(SWITCHED_TH);
      expect((host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement).disabled).toBe(true);
    });

    it('the header stays usable after the end: the language switch and the sign-out button', async () => {
      stubNetwork();
      const m = await load();
      const host = mount(m, ownsMain);
      await signedIn(host);
      m.session.sharedSession().end('expired');
      await vi.waitFor(() => {
        expect(host.querySelector('[role="alert"]')).not.toBeNull();
      });

      const bar = host.querySelector('.account-bar')!;
      expect(bar.closest('header'), 'the bar sits before the header, not in it').toBeNull();
      expect(bar.closest('fieldset'), 'the bar is outside the lock').toBeNull();
      expect(
        bar.compareDocumentPosition(host.querySelector('header')!) & Node.DOCUMENT_POSITION_FOLLOWING,
        'the bar comes before the header',
      ).toBeTruthy();
      const signOut = Array.from(bar.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent === 'ออกจากระบบ',
      );
      const langSwitch = Array.from(host.querySelectorAll<HTMLButtonElement>('header button')).find(
        (b) => b.textContent === 'EN',
      );
      expect(signOut, 'the sign-out button').toBeDefined();
      expect(langSwitch, 'the language switch').toBeDefined();
      expect(signOut!.matches(':disabled')).toBe(false);
      expect(langSwitch!.matches(':disabled')).toBe(false);
      expect(host.querySelector('.account-bar')!.textContent).toContain('Somchai');
    });

    it('in English the notice shows the en sentences, and follows a language switch', async () => {
      stubNetwork();
      const m = await load();
      m.i18n.setLang('en');
      const host = mount(m, ownsMain);
      await signedIn(host);

      const enExpired = m.en['shell.session.expired'];
      const enSwitched = m.en['shell.session.switched'];
      for (const s of [enExpired, enSwitched]) {
        expect(typeof s).toBe('string');
        expect(s.trim().length).toBeGreaterThan(0);
        expect(s).not.toMatch(THAI);
        expect(s.startsWith('shell.')).toBe(false);
      }
      expect(enExpired).not.toBe(enSwitched);

      m.session.sharedSession().end('switched');
      await vi.waitFor(() => {
        expect(host.querySelector('[role="alert"]')?.textContent).toBe(enSwitched);
      });
      m.i18n.setLang('th');
      await vi.waitFor(() => {
        expect(host.querySelector('[role="alert"]')?.textContent).toBe(SWITCHED_TH);
      });
      m.i18n.setLang('en');
      await vi.waitFor(() => {
        expect(host.querySelector('[role="alert"]')?.textContent).toBe(enSwitched);
      });
    });
  });
}

describe('AppShell lock, end to end', () => {
  it('a view read through sharedApi() answering 401 locks the shell with the expired notice', async () => {
    stubNetwork();
    const m = await load();
    const host = mount(m, false);
    await signedIn(host);
    expect((host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement).disabled).toBe(false);

    await expect(m.session.sharedApi().view('rates')).rejects.toMatchObject({ status: 401 });

    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toBe(EXPIRED_TH);
    });
    expect((host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement).disabled).toBe(true);
    for (const el of contentControls(host)) expect(el.matches(':disabled'), el.outerHTML).toBe(true);
    expect(host.querySelector('.account-bar')!.textContent).toContain('Somchai');
  });

  it('on /login (nobody signed in) a 401 on the me read locks nothing', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string) =>
        String(input).startsWith('/health/ready')
          ? json(200, READY)
          : json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }),
      ),
    );
    const m = await load();
    const host = mount(m, false);
    await vi.waitFor(() => {
      expect(host.querySelector('footer')?.textContent).toContain('v1.0.0');
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(m.session.sharedSession().ended()).toBeUndefined();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect((host.querySelector('fieldset.xc-lock') as HTMLFieldSetElement).disabled).toBe(false);
    for (const el of contentControls(host)) expect(el.matches(':disabled')).toBe(false);
  });
});
