// WE-15 (#1764): screens/admin/employees/ — show failed calls through wordedError
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { th as screenTh, en as screenEn } from './i18n';
import type { AdminBranchItem } from '../../../shared/api/generated/admin_branches';
import type { AdminEmployeeRow } from '../../../shared/api/generated/admin_employees';
import type { AdminOperationsData } from '../../../shared/api/generated/admin_operations';
import { Employees } from './Employees';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';

const SAMPLE_EMPLOYEES: AdminEmployeeRow[] = [
  {
    id: 'emp-1',
    name: 'Somchai Prasert',
    username: 'somchai',
    role: 'TELLER',
    branchId: 'B01',
    active: true,
    mustChange: false,
  },
];

const SAMPLE_BRANCHES: AdminBranchItem[] = [
  { id: 'B01', name: 'Main Branch' },
  { id: 'B02', name: 'Second Branch' },
];

const SAMPLE_OPERATIONS: AdminOperationsData = {
  entity: {
    company: '108 Exchange Co., Ltd.',
    site: 'Headquarters',
  },
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function openDialogEl(): HTMLDialogElement | null {
  return document.querySelector<HTMLDialogElement>('dialog[open]');
}

function theDialog(): HTMLDialogElement {
  const d = openDialogEl();
  expect(d, 'a dialog is open').not.toBeNull();
  return d!;
}

function findButton(root: ParentNode, texts: string[]): HTMLButtonElement {
  const found = Array.from(root.querySelectorAll<HTMLButtonElement>('button')).find((b) =>
    texts.includes(norm(b.textContent)),
  );
  expect(found, `button matching ${JSON.stringify(texts)}`).toBeDefined();
  return found!;
}

describe('Employees worded error handling (#1764)', () => {
  let container: HTMLElement;
  let dispose: (() => void) | undefined;

  let employeesHandler: () => Response | Promise<Response>;
  let operationsHandler: () => Response | Promise<Response>;
  let branchesHandler: () => Response | Promise<Response>;
  let postHandler: (url: string, body: unknown) => Response | Promise<Response>;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    toast('');

    employeesHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: { employees: SAMPLE_EMPLOYEES },
      });

    operationsHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: SAMPLE_OPERATIONS,
      });

    branchesHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: { branches: SAMPLE_BRANCHES },
      });

    postHandler = () => jsonResponse({ ok: true });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.includes('/api/v3/views/admin_employees') && method === 'GET') {
          return employeesHandler();
        }
        if (url.includes('/api/v3/views/admin_operations') && method === 'GET') {
          return operationsHandler();
        }
        if (url.includes('/api/v3/views/admin_branches') && method === 'GET') {
          return branchesHandler();
        }
        if (method === 'POST') {
          const body = init?.body ? JSON.parse(String(init.body)) : null;
          return postHandler(url, body);
        }

        return jsonResponse({ version: 1, changed: false });
      }),
    );
  });

  afterEach(() => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    container.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = 'B01') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <Employees branch={branch} period={period} shown={shown} />
            <Dialog />
            <Toast />
          </>
        ) as JSX.Element,
      container,
    );
    return { container, setBranch };
  }

  // ── 1. readEmployees ────────────────────────────────────────────────────────
  describe('catch 1: readEmployees', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      employeesHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      employeesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.serverError']);
      });
      expect(container.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      employeesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownEmpCode1764',
              message: 'Custom server employee read failure message',
            },
          },
          400,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(
          'Custom server employee read failure message',
        );
      });
    });
  });

  // ── 2. readOperations ───────────────────────────────────────────────────────
  describe('catch 2: readOperations', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      operationsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      operationsHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.serverError']);
      });
      expect(container.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      operationsHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownOpsCode1764',
              message: 'Custom server ops read failure message',
            },
          },
          400,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(
          'Custom server ops read failure message',
        );
      });
    });
  });

  // ── 3. readBranches ─────────────────────────────────────────────────────────
  describe('catch 3: readBranches', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      branchesHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      branchesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.serverError']);
      });
      expect(container.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      branchesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownBranchCode1764',
              message: 'Custom server branches read failure message',
            },
          },
          400,
        );
      mount();

      await vi.waitFor(() => {
        expect(container.textContent).toContain(
          'Custom server branches read failure message',
        );
      });
    });
  });

  // ── 4. openCreateDialog submit ──────────────────────────────────────────────
  describe('catch 4: create account dialog submit', () => {
    async function triggerCreateSubmit() {
      mount();
      await vi.waitFor(() => {
        findButton(container, [screenTh.add, screenEn.add]).click();
      });

      await vi.waitFor(() => {
        const d = theDialog();
        const nameInput = d.querySelector('input[name="name"]') as HTMLInputElement;
        const phoneInput = d.querySelector('input[name="phone"]') as HTMLInputElement;
        const passInput = d.querySelector('input[name="password"]') as HTMLInputElement;
        nameInput.value = 'New User';
        phoneInput.value = '0812345678';
        passInput.value = 'Secret1234!';
        (d.querySelector('button[type="submit"]') as HTMLButtonElement).click();
      });
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await triggerCreateSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(th['error.network']);
      });

      closeDialog();
      dispose?.();
      setLang('en');
      await triggerCreateSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      await triggerCreateSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(th['error.serverError']);
        expect(alert?.textContent).not.toContain(th['error.roleNotAllowedHere']);
      });
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownCreateCode1764',
              message: 'Custom server create failure message',
            },
          },
          400,
        );
      await triggerCreateSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain('Custom server create failure message');
      });
    });
  });

  // ── 5. openResetDialog submit ───────────────────────────────────────────────
  describe('catch 5: reset password dialog submit', () => {
    async function triggerResetSubmit() {
      mount();
      await vi.waitFor(() => {
        findButton(container, [screenTh.reset, screenEn.reset]).click();
      });

      await vi.waitFor(() => {
        const d = theDialog();
        const passInput = d.querySelector('input[name="password"]') as HTMLInputElement;
        passInput.value = 'NewSecret1234!';
        (d.querySelector('button[type="submit"]') as HTMLButtonElement).click();
      });
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await triggerResetSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(th['error.network']);
      });

      closeDialog();
      dispose?.();
      setLang('en');
      await triggerResetSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      await triggerResetSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(th['error.serverError']);
        expect(alert?.textContent).not.toContain(th['error.roleNotAllowedHere']);
      });
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownResetCode1764',
              message: 'Custom server reset failure message',
            },
          },
          400,
        );
      await triggerResetSubmit();

      await vi.waitFor(() => {
        const alert = theDialog().querySelector('[role="alert"]');
        expect(alert?.textContent).toContain('Custom server reset failure message');
      });
    });
  });

  // ── 6. handleDisable ────────────────────────────────────────────────────────
  describe('catch 6: handleDisable', () => {
    async function triggerDisable() {
      vi.spyOn(window, 'confirm').mockReturnValue(true);
      mount();
      await vi.waitFor(() => {
        findButton(container, [screenTh.disable, screenEn.disable]).click();
      });
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await triggerDisable();

      await vi.waitFor(() => {
        const toastEl = document.querySelector('[role="status"]');
        expect(toastEl?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      await triggerDisable();

      await vi.waitFor(() => {
        const toastEl = document.querySelector('[role="status"]');
        expect(toastEl?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Server internal 500 with keyed code',
            },
          },
          500,
        );
      await triggerDisable();

      await vi.waitFor(() => {
        const toastEl = document.querySelector('[role="status"]');
        expect(toastEl?.textContent).toContain(th['error.serverError']);
        expect(toastEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
      });
    });

    it('(3) refusal with unknown code and server message shows that message', async () => {
      postHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'unknownDisableCode1764',
              message: 'Custom server disable failure message',
            },
          },
          400,
        );
      await triggerDisable();

      await vi.waitFor(() => {
        const toastEl = document.querySelector('[role="status"]');
        expect(toastEl?.textContent).toContain('Custom server disable failure message');
      });
    });
  });

  // Catches 7 (openEnableDialog) and 8 (openEntityDialog) were removed, and criterion 3 now observes the
  // stored entity instead of the state line, by CA-10-FE (cash-shift-transfer.md
  // §6 Q7, §9 CA-10-FE; ticket author claude/c4d72c): the screen no longer has either dialog.

  // ── Criterion 3: Stale data cleared on branch switch before failed read ───
  describe('criterion 3: stale data isolation across branch switch', () => {
    it('clears stale operations state when operations read fails on branch switch', async () => {
      const { setBranch } = mount('B01');

      // Successful read on B01 shows employee and operations state
      await vi.waitFor(() => {
        expect(container.textContent).toContain('Somchai Prasert');
        expect(container.textContent).toContain('108 Exchange Co., Ltd.');
      });

      // Switch to B02 where operations read fails with 500
      operationsHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Failed on B02',
            },
          },
          500,
        );
      employeesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Failed on B02',
            },
          },
          500,
        );

      setBranch('B02');

      await vi.waitFor(() => {
        expect(container.textContent).toContain(th['error.serverError']);
      });

      // Stale operations state from B01 must not show (pins line 92 setOperations(null)); since CA-10-FE the
      // operations show only as the stored entity, read-only
      expect(container.textContent).not.toContain('108 Exchange Co., Ltd.');
    });

    it('hides employee table and protects against showing B01 employees when B02 employee read fails while operations succeeds', async () => {
      const { setBranch } = mount('B01');

      // Successful initial read on B01
      await vi.waitFor(() => {
        expect(container.textContent).toContain('Somchai Prasert');
        expect(container.textContent).toContain('108 Exchange Co., Ltd.');
      });

      // On branch switch to B02: operations read succeeds (B02's own entity), employee read fails with 500
      operationsHandler = () =>
        jsonResponse({
          version: 2,
          changed: true,
          data: { entity: { company: 'Second Branch Co.', site: 'Kiosk' } },
        });
      employeesHandler = () =>
        jsonResponse(
          {
            error: {
              code: 'roleNotAllowedHere',
              message: 'Failed on B02',
            },
          },
          500,
        );

      setBranch('B02');

      // Alert displays employee failure via wordedError
      await vi.waitFor(() => {
        const alert = container.querySelector('[role="alert"]');
        expect(alert?.textContent).toContain(th['error.serverError']);
      });

      // Operations side updated to B02's entity (CA-10-FE: the entity, read-only, is what the operations show)
      expect(container.textContent).toContain('Second Branch Co.');
      expect(container.textContent).not.toContain('108 Exchange Co., Ltd.');

      // Employee table container is hidden by the !employeesError() guard at line 427
      expect(container.querySelector('.table-scroll')).toBeNull();

      // Stale B01 employee must not show
      expect(container.textContent).not.toContain('Somchai Prasert');
    });
  });
});
