// T20 (#4920) up-front tests (claude/studio-claude-4, sub-controller IV-logic). Admin › Users, permission matrix
// add-group and change password check their inputs at the field (design docs/design/web-input-validation.md).
// A "line at the field" is a `[role="alert"]` inside the field's own wrapper (`.users-field` on Users and the
// matrix, `.xc-field` in PasswordField), holding a worded sentence in the screen language, never the server text.
// The implementer makes these pass and must not edit, delete or weaken them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import {
  makeAdminPermissionMatrixData,
  makeMatrixGroup,
  makeMatrixPermission,
} from '../../../shared/testing/fixtures/admin_permission_matrix';
import { ChangePassword } from '../../login/ChangePassword';
import { en } from './i18n';
import { PermissionMatrix } from './PermissionMatrix';
import { UsersScreen } from './UsersScreen';

const B01 = 'XC-B01';
const THAI = /[฀-๿]/;
const SERVER_TEXT = 'ข้อมูลไม่ถูกต้อง';

interface Call {
  path: string;
  method: string;
  body: unknown;
}

let calls: Call[] = [];
let grantResponder: () => Response;
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const ROWS = [
  makeAdminUserRow({
    userId: 'u-1',
    name: 'Somchai Active',
    status: 'ACTIVE',
    version: 1,
    roles: [makeAdminUserRole({ role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null })],
  }),
  makeAdminUserRow({
    userId: 'u-2',
    name: 'Somsri Invited',
    status: 'INVITED',
    version: 1,
    roles: [makeAdminUserRole({ role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null })],
  }),
];

const MATRIX = makeAdminPermissionMatrixData({
  version: 7,
  modules: ['FINANCE'],
  permissions: [makeMatrixPermission({ name: 'CashRead', module: 'FINANCE', level: 'VIEW', editable: true })],
  groups: [
    makeMatrixGroup({
      code: 'EXECUTIVE', name: 'EXECUTIVE', base: 'EXECUTIVE', builtIn: true, editable: false,
      permissions: ['CashRead'], levels: { FINANCE: 'VIEW' },
    }),
    makeMatrixGroup({
      code: 'TELLER', name: 'TELLER', base: 'TELLER', builtIn: true, editable: true,
      permissions: ['CashRead'], levels: { FINANCE: 'VIEW' },
    }),
  ],
});

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-15T05:00:00Z') });
  calls = [];
  grantResponder = () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 });
  document.body.innerHTML = '';
  setLang('en');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ path: u.pathname, method, body });
      if (u.pathname === '/api/v3/views/admin_branches') {
        return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom' }] } });
      }
      if (u.pathname === '/api/v3/views/admin_users') {
        return json(200, { version: 1, changed: true, data: structuredClone(ROWS) });
      }
      if (u.pathname === '/api/v3/views/admin_permission_matrix') {
        return json(200, { version: MATRIX.version, changed: true, data: MATRIX });
      }
      if (method === 'POST' && u.pathname === '/api/users/grant') return grantResponder();
      if (method === 'POST' && u.pathname === '/api/users') {
        return json(200, { userId: 'u-9', temporaryPassword: 'Temp-Pass-16char' });
      }
      if (method === 'POST' && u.pathname.startsWith('/api/admin/groups/')) {
        return json(200, { code: 'X', version: MATRIX.version + 1 });
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  await vi.runAllTimersAsync();
  await Promise.resolve();
  await Promise.resolve();
}

function host(): HTMLElement {
  const root = document.createElement('div');
  document.body.appendChild(root);
  return root;
}

function mountUsers(): HTMLElement {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const root = host();
  disposers.push(render(() => <UsersScreen branch={branch} period={period} shown={shown} />, root));
  return root;
}

function mountMatrix(): HTMLElement {
  const [shown] = createSignal(true);
  const [branch] = createSignal(B01);
  const root = host();
  disposers.push(render(() => <PermissionMatrix shown={shown} branch={branch} />, root));
  return root;
}

function el<T extends HTMLElement>(root: ParentNode, sel: string): T {
  const found = root.querySelector<T>(sel);
  expect(found, sel).not.toBeNull();
  return found!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function submit(form: HTMLFormElement): void {
  form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

function click(button: HTMLElement): void {
  button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

/** The worded line inside the input's own field wrapper, or null. */
function fieldLine(input: HTMLElement): string | null {
  const wrapper = input.closest('.users-field, .xc-field');
  expect(wrapper, 'the input sits in a field wrapper').not.toBeNull();
  const line = wrapper!.querySelector('[role="alert"]');
  const text = line?.textContent?.trim() ?? '';
  return text === '' ? null : text;
}

function expectWordedLine(input: HTMLInputElement): string {
  const text = fieldLine(input);
  expect(text, `a worded line at ${input.name || input.outerHTML}`).not.toBeNull();
  expect(text!, 'in English, never the Thai server text').not.toMatch(THAI);
  expect(text!).not.toBe(input.value);
  return text!;
}

const posts = (path: string) => calls.filter((c) => c.method === 'POST' && c.path === path);

function buttonByText(root: ParentNode, text: string): HTMLButtonElement {
  const b = Array.from(root.querySelectorAll<HTMLButtonElement>('button')).find(
    (x) => x.textContent?.trim() === text,
  );
  expect(b, `button "${text}"`).toBeDefined();
  return b!;
}

async function openGrant(root: HTMLElement): Promise<HTMLFormElement> {
  click(el(root, 'tr[data-user-id="u-1"] .users-btn-grant'));
  await settle();
  return el<HTMLFormElement>(root, '[role="dialog"] .users-dialog-form');
}

describe('T20 (#4920): admin users, matrix add-group and change password check inputs at the field', () => {
  it('U1: phone "12ab" on create and on activate gives a worded line at the phone field and sends nothing', async () => {
    const root = mountUsers();
    await settle();

    click(buttonByText(root, en.createButton));
    await settle();
    const phone = el<HTMLInputElement>(root, 'input[name="phone"]');
    expect(phone.getAttribute('type')).toBe('tel');
    expect(phone.getAttribute('inputmode')).toBe('tel');
    type(el<HTMLInputElement>(root, 'input[name="name"]'), 'New Staff');
    type(phone, '12ab');
    submit(el<HTMLFormElement>(root, '[role="dialog"] .users-dialog-form'));
    await settle();
    expect(posts('/api/users'), 'no create request').toHaveLength(0);
    expectWordedLine(phone);

    click(buttonByText(root, en.close));
    await settle();
    click(el(root, 'tr[data-user-id="u-2"] .users-btn-activate'));
    await settle();
    const activatePhone = el<HTMLInputElement>(root, 'input[name="activatePhone"]');
    expect(activatePhone.getAttribute('type')).toBe('tel');
    type(activatePhone, '12ab');
    submit(el<HTMLFormElement>(root, '[role="dialog"] .users-dialog-form'));
    await settle();
    expect(posts('/api/users'), 'no activate request').toHaveLength(0);
    expectWordedLine(activatePhone);
  });

  it('U2: validTo before validFrom gives a worded line at validTo, validTo.min follows validFrom, and sends nothing', async () => {
    const root = mountUsers();
    await settle();
    const form = await openGrant(root);
    const from = el<HTMLInputElement>(form, 'input[name="grantValidFrom"]');
    const to = el<HTMLInputElement>(form, 'input[name="grantValidTo"]');
    type(from, '2026-11-10');
    await settle();
    expect(to.getAttribute('min')).toBe('2026-11-10');
    type(to, '2026-11-01');
    submit(form);
    await settle();
    expect(posts('/api/users/grant'), 'no grant request').toHaveLength(0);
    expectWordedLine(to);
    expect(fieldLine(from), 'the line is at validTo, not validFrom').toBeNull();
  });

  it('U3: group code typed "fx_lead" shows as FX_LEAD and is sent as FX_LEAD', async () => {
    const root = mountMatrix();
    await settle();
    click(el(root, '[data-matrix-add-open]'));
    await settle();
    const code = el<HTMLInputElement>(root, 'input[data-matrix-add-code]');
    const name = el<HTMLInputElement>(root, 'input[data-matrix-add-name]');
    expect(code.maxLength).toBe(40);
    expect(name.maxLength).toBe(80);
    type(code, 'fx_lead');
    await settle();
    expect(code.value).toBe('FX_LEAD');
    type(name, 'FX lead');
    click(el(root, '[data-matrix-add-form] input[data-matrix-perm="CashRead"]'));
    await settle();
    click(el(root, '[data-matrix-add-submit]'));
    await settle();
    const added = posts('/api/admin/groups/add');
    expect(added).toHaveLength(1);
    expect((added[0]!.body as { code: string }).code).toBe('FX_LEAD');
    expect(fieldLine(code)).toBeNull();
  });

  it('U4: group code "F" gives a worded line at the code field and sends nothing', async () => {
    const root = mountMatrix();
    await settle();
    click(el(root, '[data-matrix-add-open]'));
    await settle();
    const code = el<HTMLInputElement>(root, 'input[data-matrix-add-code]');
    type(code, 'F');
    type(el<HTMLInputElement>(root, 'input[data-matrix-add-name]'), 'FX lead');
    await settle();
    click(el(root, '[data-matrix-add-submit]'));
    await settle();
    expect(posts('/api/admin/groups/add'), 'no add request').toHaveLength(0);
    expectWordedLine(code);
  });

  it('U5: an 11-character new password is refused at the field and changePassword is never called', async () => {
    const changePassword = vi.fn(async () => {});
    const onDone = vi.fn();
    const root = host();
    disposers.push(render(() => <ChangePassword auth={{ changePassword }} onDone={onDone} />, root));
    await settle();
    const next = el<HTMLInputElement>(root, 'input[name="newPassword"]');
    expect(next.getAttribute('minlength')).toBe('12');
    type(el<HTMLInputElement>(root, 'input[name="currentPassword"]'), 'Temp-Pass-16char');
    type(next, 'Abcdefgh123');
    type(el<HTMLInputElement>(root, 'input[name="confirmPassword"]'), 'Abcdefgh123');
    submit(el<HTMLFormElement>(root, 'form'));
    await settle();
    expect(changePassword).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
    expectWordedLine(next);
  });

  it('U6: a 400 validationFailed at field validTo shows "<Valid until label> is not valid" at validTo, dialog stays open', async () => {
    grantResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'validTo', message: SERVER_TEXT } });
    const root = mountUsers();
    await settle();
    const form = await openGrant(root);
    const from = el<HTMLInputElement>(form, 'input[name="grantValidFrom"]');
    const to = el<HTMLInputElement>(form, 'input[name="grantValidTo"]');
    type(from, '2026-11-01');
    type(to, '2026-12-31');
    submit(form);
    await settle();
    expect(posts('/api/users/grant')).toHaveLength(1);
    expect(root.querySelector('[role="dialog"]'), 'the dialog stays open').not.toBeNull();
    const label = el(form, 'label[for="grant-valid-to"]').textContent!.trim();
    expect(expectWordedLine(to)).toBe(`${label} is not valid`);
    expect(root.textContent).not.toContain(SERVER_TEXT);
  });
});
