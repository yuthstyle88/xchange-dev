import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
import {
  makeAdminUserRole,
  makeAdminUserRow,
  makeConsentTextsData,
  makeRetentionSetOutput,
} from '../../shared/testing/fixtures';
import { Consent } from './consent/Consent';
import * as consentTable from './consent/i18n';
import * as membersTable from './members/i18n';
import { Members } from './members/Members';
import { Toast, toast } from './parts/toast';
import * as usersTable from './users/i18n';
import { UsersScreen } from './users/UsersScreen';

const B01 = 'XC-B01';

interface SentCall {
  method: string;
  pathname: string;
  body: unknown;
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => realSetTimeout(r, 0));
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('admin-profile-bounds (#4983)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentCall[];
  let writeResponder: (call: SentCall) => Response;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    writes = [];
    writeResponder = (call) => {
      if (call.pathname === '/api/users') {
        return json(200, { userId: 'u-9', temporaryPassword: 'Temp-Pass-16char' });
      }
      if (call.pathname === '/api/users/profile') {
        return json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 });
      }
      if (call.pathname === '/api/segments') {
        return json(200, { id: 'seg-1', version: 1 });
      }
      if (call.pathname === '/api/consent-texts') {
        return json(200, { purpose: 'MARKETING', lang: 'th', version: 1 });
      }
      if (call.pathname.startsWith('/api/retention-policies/')) {
        return json(200, makeRetentionSetOutput({ dataType: 'IDENTITY_VALUE', periodDays: 3650 }));
      }
      return json(200, { ok: true });
    };

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
        return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom' }] } });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
        return json(200, {
          version: 1,
          changed: true,
          data: [
            makeAdminUserRow({
              userId: 'u-1',
              name: 'Somchai Active',
              status: 'ACTIVE',
              version: 1,
              roles: [
                makeAdminUserRole({
                  role: 'TELLER',
                  branch: B01,
                  validFrom: '2026-10-01T00:00:00Z',
                  validTo: null,
                }),
              ],
            }),
          ],
        });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/consent_texts') {
        return json(200, { version: 1, changed: true, data: makeConsentTextsData() });
      }

      if (method !== 'GET') {
        const body = typeof init?.body === 'string' && init.body ? JSON.parse(init.body) : undefined;
        const call: SentCall = { method, pathname: u.pathname, body };
        writes.push(call);
        return writeResponder(call);
      }

      return json(404, { error: { code: 'notFound', message: 'Not found' } });
    });
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    toast('');
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mountProps() {
    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    return { branch, period, shown };
  }

  // Row 1: Staff user name (1..100)
  it('Row 1: Staff user name has maxLength 100, refuses > 100 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <UsersScreen {...mountProps()} />, host);
    await settle();

    const createBtn = [...host.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === usersTable.th.createButton,
    )!;
    createBtn.click();
    await settle();

    const nameInput = host.querySelector<HTMLInputElement>('input[name="name"]')!;
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    const form = host.querySelector<HTMLFormElement>('[role="dialog"] .users-dialog-form')!;

    expect(nameInput.maxLength).toBe(100);

    typeInto(nameInput, 'A'.repeat(101));
    typeInto(phoneInput, '0812345678');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users')).toHaveLength(0);
    expect(host.querySelector('[data-field-error="name"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.name),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'name', message: 'bad' } });
    typeInto(nameInput, 'Valid Name');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users')).toHaveLength(1);
    expect(host.querySelector('[data-field-error="name"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.name),
    );
  });

  // Row 2: Employee code (0..40)
  it('Row 2: Employee code has maxLength 40, refuses > 40 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <UsersScreen {...mountProps()} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-users-action="profile"]')!.click();
    await settle();

    const form = host.querySelector<HTMLFormElement>('[data-users-form="profile"]')!;
    const input = form.querySelector<HTMLInputElement>('input[name="employeeCode"]')!;
    expect(input.maxLength).toBe(40);

    typeInto(input, 'E'.repeat(41));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(0);
    expect(form.querySelector('[data-field-error="employeeCode"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileEmployeeCode),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'employeeCode', message: 'bad' } });
    typeInto(input, 'E-01');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(1);
    expect(form.querySelector('[data-field-error="employeeCode"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileEmployeeCode),
    );
  });

  // Row 3: Department (0..80)
  it('Row 3: Department has maxLength 80, refuses > 80 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <UsersScreen {...mountProps()} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-users-action="profile"]')!.click();
    await settle();

    const form = host.querySelector<HTMLFormElement>('[data-users-form="profile"]')!;
    const input = form.querySelector<HTMLInputElement>('input[name="department"]')!;
    expect(input.maxLength).toBe(80);

    typeInto(input, 'D'.repeat(81));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(0);
    expect(form.querySelector('[data-field-error="department"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileDepartment),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'department', message: 'bad' } });
    typeInto(input, 'Operations');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(1);
    expect(form.querySelector('[data-field-error="department"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileDepartment),
    );
  });

  // Row 4: Position (0..80)
  it('Row 4: Position has maxLength 80, refuses > 80 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <UsersScreen {...mountProps()} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-users-action="profile"]')!.click();
    await settle();

    const form = host.querySelector<HTMLFormElement>('[data-users-form="profile"]')!;
    const input = form.querySelector<HTMLInputElement>('input[name="position"]')!;
    expect(input.maxLength).toBe(80);

    typeInto(input, 'P'.repeat(81));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(0);
    expect(form.querySelector('[data-field-error="position"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profilePosition),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'position', message: 'bad' } });
    typeInto(input, 'Senior Teller');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(1);
    expect(form.querySelector('[data-field-error="position"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profilePosition),
    );
  });

  // Row 5: Tell code (0..40)
  it('Row 5: Tell code has maxLength 40, refuses > 40 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <UsersScreen {...mountProps()} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-users-action="profile"]')!.click();
    await settle();

    const form = host.querySelector<HTMLFormElement>('[data-users-form="profile"]')!;
    const input = form.querySelector<HTMLInputElement>('input[name="tellCode"]')!;
    expect(input.maxLength).toBe(40);

    typeInto(input, 'T'.repeat(41));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(0);
    expect(form.querySelector('[data-field-error="tellCode"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileTellCode),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'tellCode', message: 'bad' } });
    typeInto(input, 'T-02');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/users/profile')).toHaveLength(1);
    expect(form.querySelector('[data-field-error="tellCode"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', usersTable.th.profileTellCode),
    );
  });

  // Row 6: Segment name (1..80)
  it('Row 6: Segment name has maxLength 80, refuses > 80 before send, and shows server refusal at the field', async () => {
    dispose = render(() => <Members {...mountProps()} />, host);
    await settle();

    const nameInput = host.querySelector<HTMLInputElement>('#segment-name')!;
    const defineBtn = host.querySelector<HTMLButtonElement>('button[data-action="define"]')!;
    expect(nameInput.maxLength).toBe(80);

    typeInto(nameInput, 'S'.repeat(81));
    defineBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/segments')).toHaveLength(0);
    expect(host.querySelector('[data-field-error="name"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', membersTable.th.fieldName),
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'name', message: 'bad' } });
    typeInto(nameInput, 'VIP Segment');
    defineBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/segments')).toHaveLength(1);
    expect(host.querySelector('[data-field-error="name"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', membersTable.th.fieldName),
    );
  });

  // Row 7: Segment value_thb (checkDecimal(text, MONEY): >= 0, 2 dp, <= 100,000,000)
  it('Row 7: Segment value_thb has max 100000000, checks MONEY bound before send, and shows server refusal at the field', async () => {
    dispose = render(() => <Members {...mountProps()} />, host);
    await settle();

    const nameInput = host.querySelector<HTMLInputElement>('#segment-name')!;
    const valInput = host.querySelector<HTMLInputElement>('#segment-value-thb')!;
    const defineBtn = host.querySelector<HTMLButtonElement>('button[data-action="define"]')!;
    expect(valInput.getAttribute('max')).toBe('100000000');

    typeInto(nameInput, 'High Value');
    for (const bad of ['100000000.01', '-1', '10.123', 'abc']) {
      typeInto(valInput, bad);
      defineBtn.click();
      await settle();
      expect(writes.filter((w) => w.pathname === '/api/segments'), `refused ${bad}`).toHaveLength(0);
      expect(host.querySelector('[data-field-error="rule.valueThb"]')?.textContent).toBe(
        sharedTh['input.invalid'].replace('{field}', membersTable.th.fieldValueThb),
      );
    }

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'rule.value_thb', message: 'bad' } });
    typeInto(valInput, '100000000');
    defineBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/segments')).toHaveLength(1);
    expect(host.querySelector('[data-field-error="rule.valueThb"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', membersTable.th.fieldValueThb),
    );
  });

  // Row 8: Consent body (1..20,000)
  it('Row 8: Consent body has maxLength 20000, shows n / 20000 counter, refuses > 20000 before send, and shows server refusal at the field', async () => {
    dispose = render(
      () => (
        <>
          <Toast />
          <Consent {...mountProps()} />
        </>
      ),
      host,
    );
    await settle();

    const textarea = host.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
    const publishBtn = [...host.querySelectorAll<HTMLButtonElement>('.consent-publish-form button')].find(
      (b) => b.textContent?.trim() === consentTable.th.publish,
    )!;

    expect(textarea.maxLength).toBe(20000);
    expect(host.textContent).toContain('0 / 20000');

    typeInto(textarea, 'hello');
    expect(host.textContent).toContain('5 / 20000');

    typeInto(textarea, 'x'.repeat(20001));
    publishBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/consent-texts')).toHaveLength(0);
    expect(host.querySelector('[data-field-error="body"]')?.textContent).toBe(
      consentTable.th.bodyTooLong,
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'body', message: 'bad' } });
    typeInto(textarea, 'Valid consent text');
    publishBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname === '/api/consent-texts')).toHaveLength(1);
    expect(host.querySelector('[data-field-error="body"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', consentTable.th.body),
    );
  });

  // Row 9: Retention period (days) (1..3,650)
  it('Row 9: Retention period has max 3650, refuses > 3650 before send, and shows server refusal at the field', async () => {
    dispose = render(
      () => (
        <>
          <Toast />
          <Consent {...mountProps()} />
        </>
      ),
      host,
    );
    await settle();

    const row = host.querySelector<HTMLElement>('[data-retention="IDENTITY_VALUE"]')!;
    const input = row.querySelector<HTMLInputElement>('input')!;
    const saveBtn = row.querySelector<HTMLButtonElement>('button')!;

    expect(input.getAttribute('max')).toBe('3650');

    typeInto(input, '3651');
    saveBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname.startsWith('/api/retention-policies/'))).toHaveLength(0);
    expect(row.querySelector('[data-field-error="periodDays"]')?.textContent).toBe(
      consentTable.th.periodInvalid,
    );

    writeResponder = () =>
      json(400, { error: { code: 'validationFailed', field: 'periodDays', message: 'bad' } });
    typeInto(input, '3650');
    saveBtn.click();
    await settle();

    expect(writes.filter((w) => w.pathname.startsWith('/api/retention-policies/'))).toHaveLength(1);
    expect(row.querySelector('[data-field-error="periodDays"]')?.textContent).toBe(
      sharedTh['input.invalid'].replace('{field}', consentTable.th.periodDays),
    );
  });
});
