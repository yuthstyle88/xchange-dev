import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type { AdminEmployeeRow, AdminEmployeesData } from '../../../shared/api/generated/admin_employees';
import type { AdminOperationsData } from '../../../shared/api/generated/admin_operations';
import { errorText, screenT } from '../../../shared/i18n';
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

const ROLES = [
  'FRONT',
  'BACKEND',
  'MANAGER',
  'RATES',
  'MONITOR',
  'FINANCE',
  'SYSTEM',
  'DOCUMENTS',
  'DOCUMENT_REVIEW',
] as const;

export function Employees(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [employees, setEmployees] = createSignal<AdminEmployeeRow[]>([]);
  const [employeesError, setEmployeesError] = createSignal<string | null>(null);
  const [operations, setOperations] = createSignal<AdminOperationsData | null>(null);
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [selectedBranch, setSelectedBranch] = createSignal('');

  const filteredEmployees = () => {
    const picked = selectedBranch();
    return picked ? employees().filter((e) => e.branchId === picked) : employees();
  };

  let initialized = false;
  let lastBranch = '';
  let currentOpsVersion: number | undefined = undefined;

  async function readEmployees() {
    try {
      const res = await api.view<AdminEmployeesData>('admin_employees', {
        branch: props.branch(),
      });
      if (res.data) {
        setEmployees(res.data.employees);
        setEmployeesError(null);
      }
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setEmployeesError(errorText(err.code, err.message));
      } else {
        setEmployeesError(String(err));
      }
      setEmployees([]);
    }
  }

  async function readOperations(resetCursor = false) {
    const curBranch = props.branch();
    if (resetCursor || curBranch !== lastBranch) {
      lastBranch = curBranch;
      currentOpsVersion = undefined;
    }
    try {
      const res = await api.view<AdminOperationsData>('admin_operations', {
        branch: curBranch,
        since: currentOpsVersion,
      });
      currentOpsVersion = res.version;
      if (res.changed && res.data) {
        setOperations(res.data);
      }
    } catch {
      // ignore
    }
  }

  async function readBranches() {
    try {
      const res = await api.view<AdminBranchesData>('admin_branches');
      if (res.data) {
        setBranches(res.data.branches);
      }
    } catch {
      // ignore
    }
  }

  async function reReadBoth() {
    await Promise.all([readEmployees(), readOperations(false)]);
  }

  createEffect(() => {
    if (!props.shown()) return;
    const b = props.branch();
    if (initialized && b !== lastBranch) {
      readOperations(true);
    }
  });

  createEffect(() => {
    if (!props.shown()) return;
    refreshTick();

    if (!initialized) {
      initialized = true;
      lastBranch = props.branch();
      readBranches();
      readEmployees();
      readOperations(true);
    } else {
      readEmployees();
      readOperations(false);
    }
  });

  const branchName = (branchId: string) =>
    branches().find((b) => b.id === branchId)?.name ?? branchId;

  const currentBranchName = () => branchName(props.branch());

  function openCreateDialog() {
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
          action: 'create',
          payload: {
            name: String(data.get('name') ?? ''),
            username: String(data.get('username') ?? ''),
            role: String(data.get('role') ?? 'FRONT'),
            branchId: String(data.get('branchId') ?? props.branch()),
            password: String(data.get('password') ?? ''),
          },
        });
        closeDialog();
        await reReadBoth();
      } catch (err: unknown) {
        if (err instanceof ApiError) {
          setError(errorText(err.code, err.message));
        } else {
          setError(String(err));
        }
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(t('createTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit}>
        <p>{t('handOver')}</p>
        <div class="form-grid">
          <div>
            <label for="create-name">{t('name')}</label>
            <input id="create-name" name="name" required />
          </div>
          <div>
            <label for="create-username">{t('username')}</label>
            <input id="create-username" name="username" required />
          </div>
          <div>
            <label for="create-role">{t('role')}</label>
            <select id="create-role" name="role">
              <For each={ROLES}>{(r) => <option value={r}>{r}</option>}</For>
            </select>
          </div>
          <div>
            <label for="create-branch">{t('branch')}</label>
            <select id="create-branch" name="branchId">
              <For each={branches()}>
                {(b) => (
                  <option value={b.id} selected={b.id === props.branch()}>
                    {b.name}
                  </option>
                )}
              </For>
            </select>
          </div>
          <div>
            <label for="create-password">{t('password')}</label>
            <input id="create-password" name="password" type="password" required />
          </div>
        </div>
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
        if (err instanceof ApiError) {
          setError(errorText(err.code, err.message));
        } else {
          setError(String(err));
        }
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(t('resetTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit}>
        <p>{t('sessionsEnd')}</p>
        <PasswordField
          field={passwordField}
          label={t('newPassword')}
          required
        />
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
    const ok = window.confirm(t('disableConfirm'));
    if (!ok) return;

    try {
      await api.post('/api/employees', {
        action: 'disable',
        payload: { id: employee.id },
      });
      await reReadBoth();
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        toast(errorText(err.code, err.message));
      } else {
        toast(String(err));
      }
    }
  }

  function openEnableDialog() {
    const [error, setError] = createSignal<string | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    const onSubmit = async (e: Event) => {
      e.preventDefault();
      setError(null);
      setSubmitting(true);
      try {
        await api.post('/api/employees', {
          action: 'enable',
          payload: { branchId: props.branch() },
        });
        closeDialog();
        toast(t('enabledToast'));
        await reReadBoth();
      } catch (err: unknown) {
        if (err instanceof ApiError) {
          setError(errorText(err.code, err.message));
        } else {
          setError(String(err));
        }
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(`${t('enableTitle')}${currentBranchName()}`, () => (
      <form onSubmit={onSubmit}>
        <p>{t('enableNote1')}</p>
        <p>{t('enableNote2')}</p>
        <Show when={error()}>
          <p role="alert" class="error">{error()}</p>
        </Show>
        <button type="submit" class="primary" disabled={submitting()}>
          {t('enableSubmit')}
        </button>
      </form>
    ));
  }

  function openEntityDialog() {
    let formEl: HTMLFormElement | undefined;
    const [error, setError] = createSignal<string | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    const initialCompany = operations()?.entity?.company ?? '';
    const initialSite = operations()?.entity?.site ?? '';

    const onSubmit = async (e: Event) => {
      e.preventDefault();
      if (!formEl) return;
      setError(null);
      setSubmitting(true);
      try {
        const data = new FormData(formEl);
        await api.post('/api/employees', {
          action: 'entity',
          payload: {
            branchId: props.branch(),
            company: String(data.get('company') ?? ''),
            site: String(data.get('site') ?? ''),
            confirmSeparated: true,
          },
        });
        closeDialog();
        await reReadBoth();
      } catch (err: unknown) {
        if (err instanceof ApiError) {
          setError(errorText(err.code, err.message));
        } else {
          setError(String(err));
        }
      } finally {
        setSubmitting(false);
      }
    };

    openDialog(t('entityTitle'), () => (
      <form ref={(el) => (formEl = el)} onSubmit={onSubmit}>
        <p>{t('entityNote')}</p>
        <div>
          <label for="entity-company">{t('company')}</label>
          <input id="entity-company" name="company" value={initialCompany} required />
        </div>
        <div>
          <label for="entity-site">{t('site')}</label>
          <input id="entity-site" name="site" value={initialSite} required />
        </div>
        <div>
          <label for="entity-separated">{t('separated')}</label>
          <input id="entity-separated" name="confirmSeparated" type="checkbox" required />
        </div>
        <Show when={error()}>
          <p role="alert" class="error">{error()}</p>
        </Show>
        <button type="submit" class="primary" disabled={submitting()}>
          {t('entitySubmit')}
        </button>
      </form>
    ));
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
      </Card>

      <details class="workspace-history">
        <summary>{t('workflow')}</summary>
        <div>
          <p>{operations()?.enabled ? t('stateOn') : t('stateOff')}</p>
          <div>
            <button type="button" class="secondary" onClick={openEntityDialog}>
              {t('entityButton')}
            </button>{' '}
            <button
              type="button"
              class="primary"
              disabled={operations()?.enabled === true}
              onClick={openEnableDialog}
            >
              {t('enableButton')}
            </button>
          </div>
        </div>
      </details>
    </>
  );
}
