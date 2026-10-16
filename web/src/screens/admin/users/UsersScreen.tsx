import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type { AdminUserRole, AdminUserRow } from '../../../shared/api/generated/admin_users';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { PasswordField } from '../../../shared/ui/PasswordField';
import { useForm } from '../../../shared/ui/form';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './users.css';

const t = screenT('admin.users', table);

export const AVAILABLE_ROLES = [
  'TELLER',
  'BRANCH_MANAGER',
  'RATE_MANAGER',
  'FINANCE',
  'VERIFICATION_REVIEWER',
  'ADMIN',
  'EXECUTIVE',
] as const;

export function UsersScreen(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const grantWrite = keyedWrite(api, 'POST', '/api/users/grant');
  const revokeWrite = keyedWrite(api, 'POST', '/api/users/revoke');
  const disableWrite = keyedWrite(api, 'POST', '/api/users/disable');

  const [users, setUsers] = createSignal<AdminUserRow[]>([]);
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [loading, setLoading] = createSignal<boolean>(false);
  const [pageError, setPageError] = createSignal<string | null>(null);
  const [pageSuccess, setPageSuccess] = createSignal<string | null>(null);

  // Dialog states
  const [showCreate, setShowCreate] = createSignal<boolean>(false);
  const [activatingUser, setActivatingUser] = createSignal<AdminUserRow | null>(null);
  const [grantingUser, setGrantingUser] = createSignal<AdminUserRow | null>(null);
  const [dialogError, setDialogError] = createSignal<string | null>(null);
  const [dialogSubmitting, setDialogSubmitting] = createSignal<boolean>(false);
  // Row writes (revoke ×, Disable): one in flight at a time (#2413).
  const [rowWriting, setRowWriting] = createSignal<boolean>(false);

  // Create form state
  const [createName, setCreateName] = createSignal('');
  const [createUsername, setCreateUsername] = createSignal('');
  const [createPhone, setCreatePhone] = createSignal('');
  const [createEmail, setCreateEmail] = createSignal('');
  const [createPassword, setCreatePassword] = createSignal('');
  const [createRole, setCreateRole] = createSignal<string>('TELLER');
  const [createBranch, setCreateBranch] = createSignal<string>('');

  // Activate form state
  const [activatePhone, setActivatePhone] = createSignal('');
  const [activatePassword, setActivatePassword] = createSignal('');

  // Grant form state
  const [grantRole, setGrantRole] = createSignal<string>('TELLER');
  const [grantBranch, setGrantBranch] = createSignal<string>('');

  const createForm = useForm();
  const createPasswordField = createForm.field('password');

  const activateForm = useForm();
  const activatePasswordField = activateForm.field('activatePassword');

  let branchesSeq = 0;
  let usersSeq = 0;

  const loadBranches = async () => {
    const seq = ++branchesSeq;
    try {
      const res = await api.view<AdminBranchesData>('admin_branches', {
        branch: props.branch(),
      });
      if (seq !== branchesSeq) return;
      if (res.data?.branches) {
        setBranches(res.data.branches);
      }
    } catch (err: unknown) {
      if (seq !== branchesSeq) return;
      setPageError(wordedError(err));
    }
  };

  const loadUsers = async () => {
    const seq = ++usersSeq;
    setLoading(true);
    setPageError(null);
    try {
      const res = await api.view<AdminUserRow[]>('admin_users', {
        branch: props.branch(),
      });
      if (seq !== usersSeq) return;
      if (res.data) {
        setUsers(res.data);
      }
    } catch (err: unknown) {
      if (seq !== usersSeq) return;
      setPageError(wordedError(err));
      setUsers([]);
    } finally {
      if (seq === usersSeq) {
        setLoading(false);
      }
    }
  };

  let activeBranch = '';
  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    if (activeBranch && activeBranch !== branch) {
      setUsers([]);
    }
    activeBranch = branch;
    loadBranches();
    loadUsers();
  });

  const openCreateDialog = () => {
    setCreateName('');
    setCreateUsername('');
    setCreatePhone('');
    setCreateEmail('');
    setCreatePassword('');
    setCreateRole('TELLER');
    setCreateBranch(props.branch());
    setDialogError(null);
    setShowCreate(true);
  };

  const handleCreateSubmit = async (e: Event) => {
    e.preventDefault();
    setDialogError(null);
    setDialogSubmitting(true);

    const pwdInput = document.querySelector<HTMLInputElement>('input[name="password"]');
    const pwd = pwdInput?.value || createPassword();
    setCreatePassword('');
    if (pwdInput) {
      pwdInput.value = '';
    }

    try {
      await api.post('/api/users', {
        name: createName().trim(),
        username: createUsername().trim() || null,
        phone: createPhone().trim(),
        email: createEmail().trim() || null,
        password: pwd,
        roles: [
          {
            role: createRole(),
            branch: createBranch() ? createBranch() : null,
          },
        ],
      });
      setShowCreate(false);
      setPageSuccess(t('successCreated'));
      await loadUsers();
    } catch (err: unknown) {
      setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const openActivate = (user: AdminUserRow) => {
    setActivatingUser(user);
    setActivatePhone('');
    setActivatePassword('');
    setDialogError(null);
  };

  const handleActivateSubmit = async (e: Event) => {
    e.preventDefault();
    const user = activatingUser();
    if (!user) return;

    setDialogError(null);
    setDialogSubmitting(true);

    const pwdInput = document.querySelector<HTMLInputElement>('input[name="activatePassword"]');
    const pwd = pwdInput?.value || activatePassword();
    setActivatePassword('');
    if (pwdInput) {
      pwdInput.value = '';
    }

    const roles = user.roles.map((r) => ({
      role: r.role,
      branch: r.branch,
    }));

    try {
      await api.post('/api/users', {
        name: user.name,
        phone: activatePhone().trim(),
        password: pwd,
        roles: roles,
        userId: user.userId,
      });
      setActivatingUser(null);
      setPageSuccess(t('successActivated'));
      await loadUsers();
    } catch (err: unknown) {
      setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const openGrant = (user: AdminUserRow) => {
    setGrantingUser(user);
    setGrantRole('TELLER');
    setGrantBranch(props.branch());
    setDialogError(null);
  };

  const handleGrantSubmit = async (e: Event) => {
    e.preventDefault();
    const user = grantingUser();
    if (!user) return;

    setDialogError(null);
    setDialogSubmitting(true);

    try {
      await grantWrite.send({
        userId: user.userId,
        role: grantRole(),
        branch: grantBranch() ? grantBranch() : null,
      });
      setGrantingUser(null);
      setPageSuccess(t('successGranted'));
      await loadUsers();
    } catch (err: unknown) {
      setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const handleRevoke = async (user: AdminUserRow, role: AdminUserRole) => {
    if (rowWriting()) return;
    setRowWriting(true);
    setPageError(null);
    try {
      await revokeWrite.send({
        userId: user.userId,
        role: role.role,
        branch: role.branch,
      });
      setPageSuccess(t('successRevoked'));
      await loadUsers();
    } catch (err: unknown) {
      setPageError(wordedError(err));
    } finally {
      setRowWriting(false);
    }
  };

  const handleDisable = async (user: AdminUserRow) => {
    if (rowWriting()) return;
    setRowWriting(true);
    setPageError(null);
    try {
      await disableWrite.send({
        userId: user.userId,
      });
      setPageSuccess(t('successDisabled'));
      await loadUsers();
    } catch (err: unknown) {
      setPageError(wordedError(err));
    } finally {
      setRowWriting(false);
    }
  };

  const statusClass = (status: string) => {
    switch (status.toUpperCase()) {
      case 'ACTIVE':
        return 'users-status-active';
      case 'INVITED':
        return 'users-status-invited';
      case 'PENDING':
        return 'users-status-pending';
      case 'DISABLED':
        return 'users-status-disabled';
      default:
        return '';
    }
  };

  const statusLabel = (status: string) => {
    switch (status.toUpperCase()) {
      case 'ACTIVE':
        return t('statusActive');
      case 'INVITED':
        return t('statusInvited');
      case 'PENDING':
        return t('statusPending');
      case 'DISABLED':
        return t('statusDisabled');
      default:
        return status;
    }
  };

  const activeRoles = (roles: AdminUserRole[]) =>
    roles.filter((r) => r.validTo === null);

  return (
    <div class="users-screen">
      <div class="users-header">
        <p class="users-intro">{t('intro')}</p>
      </div>

      <div class="users-toolbar">
        <div class="users-actions">
          <button
            type="button"
            class="users-btn users-btn-primary"
            onClick={openCreateDialog}
          >
            {t('createButton')}
          </button>
          <button
            type="button"
            class="users-btn"
            onClick={() => loadUsers()}
          >
            {t('refresh')}
          </button>
        </div>
      </div>

      <div class="users-content">
        <Show when={pageError()}>
          <div role="alert" class="users-form-error">{pageError()}</div>
        </Show>
        <Show when={pageSuccess()}>
          <div role="status" class="users-form-success">{pageSuccess()}</div>
        </Show>

        <div class="users-card">
          <Show
            when={users().length > 0}
            fallback={
              <div class="users-empty">
                {loading() ? t('loading') : t('noUsers')}
              </div>
            }
          >
            <div class="users-table-scroll">
              <table class="users-table">
                <thead>
                  <tr>
                    <th class="users-th">{t('colName')}</th>
                    <th class="users-th">{t('colRoles')}</th>
                    <th class="users-th">{t('colStatus')}</th>
                    <th class="users-th">{t('colActions')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={users()}>
                    {(user) => {
                      const isInvited = () =>
                        user.status === 'INVITED' || user.status === 'PENDING';
                      const isActive = () => user.status === 'ACTIVE';

                      return (
                        <tr>
                          <td class="users-td">
                            <div><strong>{user.name}</strong></div>
                            <small class="users-intro">{user.userId}</small>
                          </td>
                          <td class="users-td">
                            <div class="users-roles-list">
                              <For each={activeRoles(user.roles)}>
                                {(r) => (
                                  <span class="users-role-tag">
                                    <span class="users-role-name">{r.role}</span>
                                    <span class="users-role-branch">
                                      ({r.branch ?? t('orgWide')})
                                    </span>
                                    <Show when={isActive()}>
                                      <button
                                        type="button"
                                        class="users-btn users-btn-revoke"
                                        onClick={() => handleRevoke(user, r)}
                                        disabled={rowWriting()}
                                        aria-label={`${t('revoke')} ${r.role}`}
                                      >
                                        ×
                                      </button>
                                    </Show>
                                  </span>
                                )}
                              </For>
                            </div>
                          </td>
                          <td class="users-td">
                            <span class={`users-status-badge ${statusClass(user.status)}`}>
                              {statusLabel(user.status)}
                            </span>
                          </td>
                          <td class="users-td">
                            <div class="users-row-actions">
                              <Show when={isInvited()}>
                                <button
                                  type="button"
                                  class="users-btn users-btn-activate"
                                  onClick={() => openActivate(user)}
                                >
                                  {t('activate')}
                                </button>
                              </Show>

                              <Show when={isActive()}>
                                <button
                                  type="button"
                                  class="users-btn users-btn-grant"
                                  onClick={() => openGrant(user)}
                                >
                                  {t('grant')}
                                </button>
                                <button
                                  type="button"
                                  class="users-btn users-btn-disable"
                                  onClick={() => handleDisable(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('disable')}
                                </button>
                              </Show>
                            </div>
                          </td>
                        </tr>
                      );
                    }}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </div>
      </div>

      {/* Create User Dialog */}
      <Show when={showCreate()}>
        <div class="users-dialog-backdrop">
          <div role="dialog" aria-modal="true" class="users-dialog">
            <h3 class="users-dialog-title">{t('createTitle')}</h3>
            <Show when={dialogError()}>
              <div role="alert" class="users-form-error">{dialogError()}</div>
            </Show>
            <form class="users-dialog-form" onSubmit={handleCreateSubmit}>
              <div class="users-field">
                <label class="users-label" for="create-name">{t('name')} *</label>
                <input
                  id="create-name"
                  name="name"
                  class="users-input"
                  value={createName()}
                  onInput={(e) => setCreateName(e.currentTarget.value)}
                  required
                />
              </div>

              <div class="users-field">
                <label class="users-label" for="create-username">{t('username')}</label>
                <input
                  id="create-username"
                  name="username"
                  class="users-input"
                  value={createUsername()}
                  onInput={(e) => setCreateUsername(e.currentTarget.value)}
                />
              </div>

              <div class="users-field">
                <label class="users-label" for="create-phone">{t('phone')} *</label>
                <input
                  id="create-phone"
                  name="phone"
                  class="users-input"
                  value={createPhone()}
                  onInput={(e) => setCreatePhone(e.currentTarget.value)}
                  required
                />
              </div>

              <div class="users-field">
                <label class="users-label" for="create-email">{t('email')}</label>
                <input
                  id="create-email"
                  name="email"
                  type="email"
                  class="users-input"
                  value={createEmail()}
                  onInput={(e) => setCreateEmail(e.currentTarget.value)}
                />
              </div>

              <div class="users-field">
                <PasswordField
                  field={createPasswordField}
                  label={`${t('password')} *`}
                  required
                />
              </div>

              <div class="users-field">
                <label class="users-label" for="create-role">{t('role')} *</label>
                <select
                  id="create-role"
                  name="role"
                  class="users-select"
                  value={createRole()}
                  onChange={(e) => setCreateRole(e.currentTarget.value)}
                >
                  <For each={AVAILABLE_ROLES}>
                    {(r) => <option value={r}>{r}</option>}
                  </For>
                </select>
              </div>

              <div class="users-field">
                <label class="users-label" for="create-branch">{t('branch')}</label>
                <select
                  id="create-branch"
                  name="branch"
                  class="users-select"
                  value={createBranch()}
                  onChange={(e) => setCreateBranch(e.currentTarget.value)}
                >
                  <option value="">{t('orgWide')}</option>
                  <For each={branches()}>
                    {(b) => <option value={b.id}>{b.name} ({b.id})</option>}
                  </For>
                </select>
              </div>

              <div class="users-dialog-actions">
                <button
                  type="button"
                  class="users-btn users-btn-secondary"
                  onClick={() => setShowCreate(false)}
                >
                  {t('close')}
                </button>
                <button
                  type="submit"
                  class="users-btn users-btn-primary"
                  disabled={dialogSubmitting()}
                >
                  {dialogSubmitting() ? t('creating') : t('createSubmit')}
                </button>
              </div>
            </form>
          </div>
        </div>
      </Show>

      {/* Activate Dialog */}
      <Show when={activatingUser()}>
        {(user) => (
          <div class="users-dialog-backdrop">
            <div role="dialog" aria-modal="true" class="users-dialog">
              <h3 class="users-dialog-title">{t('activateTitle')}: {user().name}</h3>
              <p class="users-dialog-intro">{t('activateIntro')}</p>
              <Show when={dialogError()}>
                <div role="alert" class="users-form-error">{dialogError()}</div>
              </Show>
              <form class="users-dialog-form" onSubmit={handleActivateSubmit}>
                <div class="users-field">
                  <label class="users-label" for="activate-phone">{t('phone')} *</label>
                  <input
                    id="activate-phone"
                    name="activatePhone"
                    class="users-input"
                    value={activatePhone()}
                    onInput={(e) => setActivatePhone(e.currentTarget.value)}
                    required
                  />
                </div>

                <div class="users-field">
                  <PasswordField
                    field={activatePasswordField}
                    label={`${t('password')} *`}
                    required
                  />
                </div>

                <div class="users-dialog-actions">
                  <button
                    type="button"
                    class="users-btn users-btn-secondary"
                    onClick={() => setActivatingUser(null)}
                  >
                    {t('close')}
                  </button>
                  <button
                    type="submit"
                    class="users-btn users-btn-activate"
                    disabled={dialogSubmitting()}
                  >
                    {dialogSubmitting() ? t('activating') : t('activateSubmit')}
                  </button>
                </div>
              </form>
            </div>
          </div>
        )}
      </Show>

      {/* Grant Role Dialog */}
      <Show when={grantingUser()}>
        {(user) => (
          <div class="users-dialog-backdrop">
            <div role="dialog" aria-modal="true" class="users-dialog">
              <h3 class="users-dialog-title">{t('grantTitle')}: {user().name}</h3>
              <Show when={dialogError()}>
                <div role="alert" class="users-form-error">{dialogError()}</div>
              </Show>
              <form class="users-dialog-form" onSubmit={handleGrantSubmit}>
                <div class="users-field">
                  <label class="users-label" for="grant-role">{t('role')} *</label>
                  <select
                    id="grant-role"
                    name="grantRole"
                    class="users-select"
                    value={grantRole()}
                    onChange={(e) => setGrantRole(e.currentTarget.value)}
                  >
                    <For each={AVAILABLE_ROLES}>
                      {(r) => <option value={r}>{r}</option>}
                    </For>
                  </select>
                </div>

                <div class="users-field">
                  <label class="users-label" for="grant-branch">{t('branch')}</label>
                  <select
                    id="grant-branch"
                    name="grantBranch"
                    class="users-select"
                    value={grantBranch()}
                    onChange={(e) => setGrantBranch(e.currentTarget.value)}
                  >
                    <option value="">{t('orgWide')}</option>
                    <For each={branches()}>
                      {(b) => <option value={b.id}>{b.name} ({b.id})</option>}
                    </For>
                  </select>
                </div>

                <div class="users-dialog-actions">
                  <button
                    type="button"
                    class="users-btn users-btn-secondary"
                    onClick={() => setGrantingUser(null)}
                  >
                    {t('close')}
                  </button>
                  <button
                    type="submit"
                    class="users-btn users-btn-primary"
                    disabled={dialogSubmitting()}
                  >
                    {dialogSubmitting() ? t('granting') : t('grantSubmit')}
                  </button>
                </div>
              </form>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
