import {
  createEffect,
  createSignal,
  For,
  Show,
  untrack,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type { AdminEmployeeRow, AdminEmployeesData } from '../../../shared/api/generated/admin_employees';
import type { AdminOperationsData } from '../../../shared/api/generated/admin_operations';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Card } from '../../../shared/ui/Card';
import { PasswordField } from '../../../shared/ui/PasswordField';
import { useForm } from '../../../shared/ui/form';
import type { AdminMenuProps } from '../menus';
import { closeDialog, openDialog } from '../parts/Dialog';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './employees.css';

const t = screenT('admin.employees', table);

// #1157: the roles a shop owner gives a staff member at a branch (`userCreate`, ScopeRule allows a branch
// for each). The account is created in Identity server-side; the person signs in with the phone.
const ROLES = [
  'TELLER',
  'BRANCH_MANAGER',
  'RATE_MANAGER',
  'FINANCE',
  'VERIFICATION_REVIEWER',
  'ADMIN',
] as const;

export function Employees(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [employees, setEmployees] = createSignal<AdminEmployeeRow[]>([]);
  const [accountsError, setAccountsError] = createSignal<string | null>(null);
  const [operationsError, setOperationsError] = createSignal<string | null>(null);
  const [branchesError, setBranchesError] = createSignal<string | null>(null);
  const employeesError = () => accountsError() ?? branchesError();
  const setEmployeesError = (err: string | null) => setAccountsError(err);
  const [operations, setOperations] = createSignal<AdminOperationsData | null>(null);
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [selectedBranch, setSelectedBranch] = createSignal('');
  const [disabling, setDisabling] = createSignal(false);

  const filteredEmployees = () => {
    const picked = selectedBranch();
    return picked ? employees().filter((e) => e.branchId === picked) : employees();
  };

  let initialized = false;
  let lastBranch = '';
  let currentOpsVersion: number | undefined = undefined;
  let gen = 0;
  let employeesSeq = 0;
  let operationsSeq = 0;
  let branchesSeq = 0;

  async function readEmployees() {
    const branch = untrack(() => props.branch());
    const myGen = gen;
    const mySeq = ++employeesSeq;
    try {
      const res = await api.view<AdminEmployeesData>('admin_employees', {
        branch,
      });
      if (myGen !== gen || mySeq !== employeesSeq) return;
      if (res.data) {
        setEmployees(res.data.employees);
        setEmployeesError(null);
      }
    } catch (err: unknown) {
      if (myGen !== gen || mySeq !== employeesSeq) return;
      setEmployeesError(wordedError(err));
      setEmployees([]);
    }
  }

  async function readOperations(resetCursor = false) {
    const curBranch = untrack(() => props.branch());
    if (resetCursor || curBranch !== lastBranch) {
      lastBranch = curBranch;
      currentOpsVersion = undefined;
    }
    const myGen = gen;
    const mySeq = ++operationsSeq;
    try {
      const res = await api.view<AdminOperationsData>('admin_operations', {
        branch: curBranch,
        since: currentOpsVersion,
      });
      if (myGen !== gen || mySeq !== operationsSeq) return;
      currentOpsVersion = res.version;
      setOperationsError(null);
      if (res.changed && res.data) {
        setOperations(res.data);
      }
    } catch (err: unknown) {
      if (myGen !== gen || mySeq !== operationsSeq) return;
      setOperationsError(wordedError(err));
      setOperations(null);
    }
  }

  async function readBranches() {
    const mySeq = ++branchesSeq;
    try {
      const res = await api.view<AdminBranchesData>('admin_branches');
      if (mySeq !== branchesSeq) return;
      setBranchesError(null);
      if (res.data) {
        setBranches(res.data.branches);
      }
    } catch (err: unknown) {
      if (mySeq !== branchesSeq) return;
      setBranchesError(wordedError(err));
    }
  }

  async function reReadBoth() {
    await Promise.all([readEmployees(), readOperations(false)]);
  }

  createEffect(() => {
    if (!props.shown()) return;
    const b = props.branch();
    refreshTick();

    if (!initialized) {
      initialized = true;
      lastBranch = b;
      gen++;
      readBranches();
      readEmployees();
      readOperations(true);
    } else if (b !== lastBranch) {
      lastBranch = b;
      currentOpsVersion = undefined;
      gen++;
      readEmployees();
      readOperations(true);
    } else {
      if (untrack(branchesError)) {
        readBranches();
      }
      readEmployees();
      readOperations(false);
    }
  });

  const branchName = (branchId: string) =>
    branches().find((b) => b.id === branchId)?.name ?? branchId;

  function openCreateDialog() {
    const form = useForm();
    const passwordField = form.field('password');
    let formEl: HTMLFormElement | undefined;
    const [error, setError] = createSignal<string | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    const onSubmit = async (e: Event) => {
      e.preventDefault();
      if (!formEl) return;
      setError(null);
      setSubmitting(true);
      try {
        const data = new FormData(formEl);
        const email = String(data.get('email') ?? '').trim();
        const username = String(data.get('username') ?? '').trim();
        await api.post('/api/users', {
          name: String(data.get('name') ?? ''),
          username: username === '' ? null : username,
          phone: String(data.get('phone') ?? '').trim(),
          email: email === '' ? null : email,
          password: String(data.get('password') ?? ''),
          roles: [
            {
              role: String(data.get('role') ?? 'TELLER'),
              branch: String(data.get('branchId') ?? props.branch()),
            },
          ],
        });
        closeDialog();
        await reReadBoth();
      } catch (err: unknown) {
        setError(wordedError(err));
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(t('createTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit}>
        <div class="form-grid">
          <label for="create-name">
            {t('name')}
            <input id="create-name" name="name" required />
          </label>
          <label for="create-phone">
            {t('phone')}
            <input id="create-phone" name="phone" type="tel" autocomplete="off" required />
          </label>
          <label for="create-username">
            {t('username')}
            <input id="create-username" name="username" />
          </label>
          <label for="create-email">
            {t('email')}
            <input id="create-email" name="email" type="email" />
          </label>
          <label for="create-role">
            {t('role')}
            <select id="create-role" name="role">
              <For each={ROLES}>{(r) => <option value={r}>{r}</option>}</For>
            </select>
          </label>
          <label for="create-branch">
            {t('branch')}
            <select id="create-branch" name="branchId">
              <For each={branches()}>
                {(b) => (
                  <option value={b.id} selected={b.id === props.branch()}>
                    {b.name}
                  </option>
                )}
              </For>
            </select>
          </label>
          <PasswordField
            field={passwordField}
            label={t('password')}
            required
          />
        </div>
        <p>{t('handOver')}</p>
        <Show when={error()}>
          <p role="alert" class="error">{error()}</p>
        </Show>
        <button type="submit" class="primary" disabled={submitting()}>
          {t('createSubmit')}
        </button>
      </form>
    ));
  }

  function openResetDialog(employee: AdminEmployeeRow) {
    const form = useForm();
    const passwordField = form.field('password');
    let formEl: HTMLFormElement | undefined;
    const [error, setError] = createSignal<string | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    const onSubmit = async (e: Event) => {
      e.preventDefault();
      if (!formEl) return;
      setError(null);
      setSubmitting(true);
      try {
        const data = new FormData(formEl);
        await api.post('/api/employees', {
          action: 'reset',
          payload: {
            id: employee.id,
            password: String(data.get('password') ?? ''),
          },
        });
        closeDialog();
        await reReadBoth();
      } catch (err: unknown) {
        setError(wordedError(err));
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(t('resetTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit}>
        <PasswordField
          field={passwordField}
          label={t('newPassword')}
          required
        />
        <p>{t('sessionsEnd')}</p>
        <Show when={error()}>
          <p role="alert" class="error">{error()}</p>
        </Show>
        <button type="submit" class="primary" disabled={submitting()}>
          {t('resetSubmit')}
        </button>
      </form>
    ));
  }

  async function handleDisable(employee: AdminEmployeeRow) {
    if (disabling()) return;
    const ok = window.confirm(t('disableConfirm'));
    if (!ok) return;

    setDisabling(true);
    try {
      await api.post('/api/employees', {
        action: 'disable',
        payload: { id: employee.id },
      });
      await reReadBoth();
    } catch (err: unknown) {
      toast(wordedError(err));
    } finally {
      setDisabling(false);
    }
  }

  return (
    <>
      <Card class="employees-card">
        <header>
          <h2>{t('heading')}</h2>
          <p>{t('intro')}</p>
          <div>
            <button type="button" class="primary" onClick={openCreateDialog}>
              {t('add')}
            </button>{' '}
            <button type="button" class="secondary" onClick={() => readEmployees()}>
              {t('refresh')}
            </button>{' '}
            <label class="employees-filter">
              <span>{t('filterBranch')}</span>{' '}
              <select
                aria-label={t('filterBranch')}
                value={selectedBranch()}
                onChange={(e) => setSelectedBranch(e.currentTarget.value)}
              >
                <option value="">{t('filterAll')}</option>
                <For each={branches()}>
                  {(b) => <option value={b.id}>{branchName(b.id)}</option>}
                </For>
              </select>
            </label>{' '}
            <a href="/staff" target="_blank" rel="noopener">
              {t('staffLink')}
            </a>
          </div>
        </header>

        <Show when={employeesError()}>
          <div role="alert">{employeesError()}</div>
        </Show>

        <Show when={!employeesError() && operationsError()}>
          <div role="alert">{operationsError()}</div>
        </Show>

        <Show when={!employeesError()}>
          <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{t('headerAccountName')}</th>
                <th>{t('headerRole')}</th>
                <th>{t('headerBranch')}</th>
                <th>{t('headerStatus')}</th>
                <th>{t('headerActions')}</th>
              </tr>
            </thead>
            <tbody>
              <Show
                when={filteredEmployees().length > 0}
                fallback={
                  <Show when={!employeesError()}>
                    <tr>
                      <td colspan="5">{t('filterEmpty')}</td>
                    </tr>
                  </Show>
                }
              >
                <For each={filteredEmployees()}>
                  {(e) => {
                    const statusText = () => {
                      if (!e.active) return t('disabled');
                      if (e.mustChange) return t('mustChange');
                      return t('active');
                    };

                    return (
                      <tr>
                        <td>
                          <div>{e.username}</div>
                          <div class="cell-sub">{e.name}</div>
                        </td>
                        <td>{e.role}</td>
                        <td>{branchName(e.branchId)}</td>
                        <td>{statusText()}</td>
                        <td>
                          <Show when={e.active}>
                            <button
                              type="button"
                              class="secondary"
                              onClick={() => openResetDialog(e)}
                            >
                              {t('reset')}
                            </button>{' '}
                            <button
                              type="button"
                              class="danger"
                              disabled={disabling()}
                              onClick={() => handleDisable(e)}
                            >
                              {t('disable')}
                            </button>
                          </Show>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </Show>
            </tbody>
          </table>
        </div>
        </Show>
      </Card>

      <Show when={operations()?.entity}>
        {(entity) => (
          <p>{`${entity().company} · ${entity().site}`}</p>
        )}
      </Show>
    </>
  );
}
