// OR-7.7 (#1030 §3.10, §7; lane controller's ruling (a), issuecomment-6105567502, and the FE controller's
// two checks relayed by claude/ad05bf8d): the account bar's role part, once `me` carries `roles` and no
// longer `role`/`username`.
// Written before the implementation by the test-writer claude/5d45b017. Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The rule pinned here ([required]):
//   - the line is `{name} · {role part}`; the role part is the label of each row of `me.roles` that holds
//     at `me.branch` (its `branchId` is `me.branch`, or null: organisation-wide), in `roles`' order,
//     joined with ", "; with no name, the role part alone;
//   - a label is the shared table's `shell.account.role.<camelCase>` in th, en and zh. Every role code of
//     `Role::as_str` (api/crates/xc-auth/src/domain/access/role.rs) has one, and no raw role code is
//     shown for those;
//   - PARTNER (Broker-Platform) has no key; a code with no key is shown as the raw code, never an
//     empty label.
// The wording is FE's: every expectation reads the tables, none hard-codes a label. Access never follows
// a label (FE check 2): nothing here decides visibility.
import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import type { SessionStore } from '../session/session';
import { AccountBar } from './AccountBar';
import { roleLabel, type MeRole } from '../../test-support/me';

/** `Role::as_str` → its key's camelCase segment. */
const CODES: Record<string, string> = {
  EXECUTIVE: 'executive',
  ADMIN: 'admin',
  RATE_MANAGER: 'rateManager',
  BRANCH_MANAGER: 'branchManager',
  TELLER: 'teller',
  FINANCE: 'finance',
  MARKETING: 'marketing',
  CUSTOMER: 'customer',
  VERIFICATION_REVIEWER: 'verificationReviewer',
};

const TABLES: Record<string, Record<string, string>> = {
  th: th as unknown as Record<string, string>,
  en: en as unknown as Record<string, string>,
  zh: zh as unknown as Record<string, string>,
};

function me(roles: MeRole[], name: string | null = 'Nok', branch = 'B2'): Me {
  return {
    id: 'u-7',
    name,
    roles,
    permissions: [],
    branch,
    branchName: 'สาขาสอง',
    branches: [{ id: branch, code: 'XC-02', name: 'สาขาสอง' }],
  } as unknown as Me;
}

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

function mount(initial: Me) {
  const [get, set] = createSignal<Me | undefined>(initial);
  const session: SessionStore = { me: get, signOut: async () => undefined, dispose: () => undefined };
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <AccountBar session={session} navigate={() => undefined} />, host);
  return { host, setMe: (m: Me) => set(() => m) };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ');
}

function label(code: string): string {
  const l = roleLabel(CODES[code]!);
  expect(l, `shell.account.role.${CODES[code]} exists in th`).toBeTruthy();
  return l;
}

describe('OR-7.7 the role labels in the shared tables', () => {
  it('every role code of Role::as_str has a non-empty shell.account.role.<camelCase> in th, en and zh', () => {
    for (const [code, camel] of Object.entries(CODES)) {
      for (const [lang, table] of Object.entries(TABLES)) {
        const v = table[`shell.account.role.${camel}`];
        expect(typeof v === 'string' && v.trim() !== '', `${lang} shell.account.role.${camel} (${code})`).toBe(true);
      }
    }
  });

  it('PARTNER has no key in any table', () => {
    for (const [lang, table] of Object.entries(TABLES)) {
      expect(Object.keys(table).filter((k) => /^shell\.account\.role\.partner$/i.test(k)), lang).toEqual([]);
    }
  });
});

describe('OR-7.7 the account line: name, then the labels of the roles held at me.branch', () => {
  it('a TELLER at B2: "Nok · <teller>", no raw code, no username', async () => {
    const { host } = mount(me([{ role: 'TELLER', branchId: 'B2' }]));
    await settle();
    expect(text(host)).toContain(`Nok · ${label('TELLER')}`);
    expect(text(host)).not.toMatch(/\bTELLER\b/);
  });

  it('two roles at me.branch: both labels, in roles\' order, joined with ", "', async () => {
    const { host } = mount(me([{ role: 'TELLER', branchId: 'B2' }, { role: 'FINANCE', branchId: 'B2' }]));
    await settle();
    expect(text(host)).toContain(`Nok · ${label('TELLER')}, ${label('FINANCE')}`);
  });

  it('a role held only at another branch adds nothing to the line', async () => {
    const { host, setMe } = mount(me([{ role: 'TELLER', branchId: 'B2' }]));
    await settle();
    const alone = text(host);
    expect(alone).toContain(`Nok · ${label('TELLER')}`);
    setMe(me([{ role: 'TELLER', branchId: 'B2' }, { role: 'FINANCE', branchId: 'B3' }]));
    await settle();
    expect(text(host)).toBe(alone);
  });

  it('an organisation-wide role (branchId null) holds at me.branch', async () => {
    const { host } = mount(me([{ role: 'ADMIN', branchId: null }, { role: 'EXECUTIVE', branchId: null }]));
    await settle();
    expect(text(host)).toContain(`Nok · ${label('ADMIN')}, ${label('EXECUTIVE')}`);
    expect(text(host)).not.toMatch(/\b(ADMIN|EXECUTIVE)\b/);
  });

  it('no name: the role part alone, with no "·" and no "null"', async () => {
    const { host } = mount(me([{ role: 'TELLER', branchId: 'B2' }], null));
    await settle();
    expect(text(host)).toContain(label('TELLER'));
    expect(text(host)).not.toContain('·');
    expect(text(host)).not.toContain('null');
  });

  it('every known code shows its label and never the raw code (FE check 1), in th and in en', async () => {
    for (const lang of ['th', 'en'] as const) {
      setLang(lang);
      for (const [code, camel] of Object.entries(CODES)) {
        const { host } = mount(me([{ role: code, branchId: 'B2' }]));
        await settle();
        const want = roleLabel(camel, TABLES[lang]);
        expect(want, `${lang} label of ${code}`).toBeTruthy();
        expect(text(host), `${lang} ${code}`).toContain(`Nok · ${want}`);
        expect(text(host), `${lang} ${code}`).not.toMatch(new RegExp(`\\b${code}\\b`));
        dispose?.();
        document.body.innerHTML = '';
      }
    }
  });

  it('a code with no key (PARTNER, or one the web does not know yet) shows the raw code, never an empty label', async () => {
    for (const code of ['PARTNER', 'SOMETHING_NEW']) {
      const { host } = mount(me([{ role: 'TELLER', branchId: 'B2' }, { role: code, branchId: 'B2' }]));
      await settle();
      expect(text(host), code).toContain(`Nok · ${label('TELLER')}, ${code}`);
      dispose?.();
      document.body.innerHTML = '';
    }
  });
});
