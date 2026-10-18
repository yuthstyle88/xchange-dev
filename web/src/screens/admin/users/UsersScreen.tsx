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
import type { UserChangeOutput } from '../../../shared/api/generated/user_disable';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { PasswordField } from '../../../shared/ui/PasswordField';
import { useForm } from '../../../shared/ui/form';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './users.css';

const t = screenT('admin.users', table);

export interface UsersTabProps {
  shown: () => boolean;
  branch: () => string;
}

type UsersTabComponent = (props: UsersTabProps) => JSX.Element;

const TAB_MODULES = import.meta.glob<Record<string, unknown>>(
  './{PermissionMatrix,UserAudit,LoginHistory,RegisterImport}.tsx',
  { eager: true },
);

function resolveTabComponent(name: string): UsersTabComponent | null {
  for (const [path, mod] of Object.entries(TAB_MODULES)) {
    if (path.endsWith(`/${name}.tsx`) && mod && typeof mod[name] === 'function') {
      return mod[name] as UsersTabComponent;
    }
  }
  return null;
}

const TABS = [
  { id: 'users', labelKey: 'tabUsers', file: null },
  { id: 'matrix', labelKey: 'tabMatrix', file: 'PermissionMatrix' },
  { id: 'audit', labelKey: 'tabAudit', file: 'UserAudit' },
  { id: 'logins', labelKey: 'tabLogins', file: 'LoginHistory' },
  { id: 'import', labelKey: 'tabImport', file: 'RegisterImport' },
] as const;
type UsersTabId = (typeof TABS)[number]['id'];

// moves to generated/user_reset_password.ts when C1/F2 merges
type UserResetPasswordOutput = { userId: string; temporaryPassword: string };
// moves to generated/user_profile_save.ts when C1/F3 merges
type UserProfileSaveInput = {
  userId: string;
  employeeCode: string | null;
  department: string | null;
  position: string | null;
  tellCode: string | null;
};
// moves to generated/admin_users.ts when a BE ticket adds these fields to the admin_users view (no ticket yet; gap on xchange#564)
type AdminUserProfile = {
  employeeCode?: string | null;
  department?: string | null;
  position?: string | null;
  tellCode?: string | null;
};
type UserRow = AdminUserRow & AdminUserProfile;

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
  const disableWrite = keyedWrite<{ userId: string }, UserChangeOutput>(api, 'POST', '/api/users/disable');
  const suspendWrite = keyedWrite<{ userId: string }, UserChangeOutput>(api, 'POST', '/api/users/suspend');
  const resumeWrite = keyedWrite<{ userId: string }, UserChangeOutput>(api, 'POST', '/api/users/resume');
  const enableWrite = keyedWrite<{ userId: string }, UserChangeOutput>(api, 'POST', '/api/users/enable');
  const resetPasswordWrite = keyedWrite<{ userId: string }, UserResetPasswordOutput>(api, 'POST', '/api/users/reset-password');
  const profileWrite = keyedWrite<UserProfileSaveInput, UserChangeOutput>(api, 'POST', '/api/users/profile');

  const [activeTab, setActiveTab] = createSignal<UsersTabId>('users');
  const [users, setUsers] = createSignal<UserRow[]>([]);
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [loading, setLoading] = createSignal<boolean>(false);
  const [pageError, setPageError] = createSignal<string | null>(null);
  const [pageSuccess, setPageSuccess] = createSignal<string | null>(null);
  const [tempPassword, setTempPassword] = createSignal<string | null>(null);

  // Dialog states
  const [showCreate, setShowCreate] = createSignal<boolean>(false);
  const [activatingUser, setActivatingUser] = createSignal<AdminUserRow | null>(null);
  const [grantingUser, setGrantingUser] = createSignal<AdminUserRow | null>(null);
  const [profilingUser, setProfilingUser] = createSignal<UserRow | null>(null);
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

  // Profile form state
  const [profileEmployeeCode, setProfileEmployeeCode] = createSignal('');
  const [profileDepartment, setProfileDepartment] = createSignal('');
  const [profilePosition, setProfilePosition] = createSignal('');
  const [profileTellCode, setProfileTellCode] = createSignal('');

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
      const res = await api.view<UserRow[]>('admin_users', {
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
      setPageSuccess(null);
      setTempPassword(null);
    }
    activeBranch = branch;
    loadBranches();
    loadUsers();
  });

  // Which dialog is open (kind + user id). A dialog write's late answer only
  // closes / sets the error of the dialog it was sent from (#2460).
  // Each opening also gets a fresh generation, so a dialog of the same kind and
  // user that was closed and reopened is not the one the write was sent from (#2472).
  let dialogOpening = 0;
  const openDialogKey = (): string | null => {
    if (showCreate()) return `create#${dialogOpening}`;
    const activating = activatingUser();
    if (activating) return `activate:${activating.userId}#${dialogOpening}`;
    const granting = grantingUser();
    if (granting) return `grant:${granting.userId}#${dialogOpening}`;
    const profiling = profilingUser();
    if (profiling) return `profile:${profiling.userId}#${dialogOpening}`;
    return null;
  };

  const openCreateDialog = () => {
    dialogOpening++;
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
    const sentFrom = openDialogKey();
    setDialogError(null);
    setPageSuccess(null);
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
      if (openDialogKey() === sentFrom) setShowCreate(false);
      setPageSuccess(t('successCreated'));
      await loadUsers();
    } catch (err: unknown) {
      if (openDialogKey() === sentFrom) setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const openActivate = (user: AdminUserRow) => {
    dialogOpening++;
    setActivatingUser(user);
    setActivatePhone('');
    setActivatePassword('');
    setDialogError(null);
  };

  const handleActivateSubmit = async (e: Event) => {
    e.preventDefault();
    const user = activatingUser();
    if (!user) return;
    const sentFrom = openDialogKey();

    setDialogError(null);
    setPageSuccess(null);
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
      if (openDialogKey() === sentFrom) setActivatingUser(null);
      setPageSuccess(t('successActivated'));
      await loadUsers();
    } catch (err: unknown) {
      if (openDialogKey() === sentFrom) setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const openGrant = (user: AdminUserRow) => {
    dialogOpening++;
    setGrantingUser(user);
    setGrantRole('TELLER');
    setGrantBranch(props.branch());
    setDialogError(null);
  };

  const handleGrantSubmit = async (e: Event) => {
    e.preventDefault();
    const user = grantingUser();
    if (!user) return;
    const sentFrom = openDialogKey();

    setDialogError(null);
    setPageSuccess(null);
    setDialogSubmitting(true);

    try {
      await grantWrite.send({
        userId: user.userId,
        role: grantRole(),
        branch: grantBranch() ? grantBranch() : null,
      });
      if (openDialogKey() === sentFrom) setGrantingUser(null);
      setPageSuccess(t('successGranted'));
      await loadUsers();
    } catch (err: unknown) {
      if (openDialogKey() === sentFrom) setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const openProfile = (user: UserRow) => {
    dialogOpening++;
    setProfilingUser(user);
    setProfileEmployeeCode(user.employeeCode ?? '');
    setProfileDepartment(user.department ?? '');
    setProfilePosition(user.position ?? '');
    setProfileTellCode(user.tellCode ?? '');
    setDialogError(null);
  };

  const handleProfileSubmit = async (e: Event) => {
    e.preventDefault();
    const user = profilingUser();
    if (!user) return;
    const sentFrom = openDialogKey();

    setDialogError(null);
    setPageSuccess(null);
    setDialogSubmitting(true);

    const trimOrNull = (val: string): string | null => {
      const trimmed = val.trim();
      return trimmed === '' ? null : trimmed;
    };

    try {
      await profileWrite.send({
        userId: user.userId,
        employeeCode: trimOrNull(profileEmployeeCode()),
        department: trimOrNull(profileDepartment()),
        position: trimOrNull(profilePosition()),
        tellCode: trimOrNull(profileTellCode()),
      });
      if (openDialogKey() === sentFrom) setProfilingUser(null);
      setPageSuccess(t('successProfileSaved'));
      await loadUsers();
    } catch (err: unknown) {
      if (openDialogKey() === sentFrom) setDialogError(wordedError(err));
    } finally {
      setDialogSubmitting(false);
    }
  };

  const handleRevoke = async (user: AdminUserRow, role: AdminUserRole) => {
    if (rowWriting()) return;
    setRowWriting(true);
    setPageError(null);
    setPageSuccess(null);
    const branch = props.branch();
    try {
      await revokeWrite.send({
        userId: user.userId,
        role: role.role,
        branch: role.branch,
      });
      if (props.branch() !== branch) return;
      setPageSuccess(t('successRevoked'));
      await loadUsers();
    } catch (err: unknown) {
      if (props.branch() !== branch) return;
      setPageSuccess(null);
      setPageError(wordedError(err));
    } finally {
      setRowWriting(false);
    }
  };

  const handleStatusWrite = async (
    write: { send(body: { userId: string }): Promise<UserChangeOutput> },
    user: AdminUserRow,
    successKey: 'successDisabled' | 'successSuspended' | 'successResumed' | 'successEnabled',
  ) => {
    if (rowWriting()) return;
    setRowWriting(true);
    setPageError(null);
    setPageSuccess(null);
    const branch = props.branch();
    try {
      await write.send({
        userId: user.userId,
      });
      if (props.branch() !== branch) return;
      setPageSuccess(t(successKey));
      await loadUsers();
    } catch (err: unknown) {
      if (props.branch() !== branch) return;
      setPageSuccess(null);
      setPageError(wordedError(err));
    } finally {
      setRowWriting(false);
    }
  };

  const handleDisable = (user: AdminUserRow) =>
    handleStatusWrite(disableWrite, user, 'successDisabled');
  const handleSuspend = (user: AdminUserRow) =>
    handleStatusWrite(suspendWrite, user, 'successSuspended');
  const handleResume = (user: AdminUserRow) =>
    handleStatusWrite(resumeWrite, user, 'successResumed');
  const handleEnable = (user: AdminUserRow) =>
    handleStatusWrite(enableWrite, user, 'successEnabled');

  const handleResetPassword = async (user: AdminUserRow) => {
    if (rowWriting()) return;
    setRowWriting(true);
    setPageError(null);
    setPageSuccess(null);
    setTempPassword(null);
    const branch = props.branch();
    try {
      const res = await resetPasswordWrite.send({
        userId: user.userId,
      });
      if (props.branch() !== branch) return;
      setTempPassword(res.temporaryPassword);
    } catch (err: unknown) {
      if (props.branch() !== branch) return;
      setPageSuccess(null);
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
      case 'SUSPENDED':
        return 'users-status-suspended';
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
      case 'SUSPENDED':
        return t('statusSuspended');
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
        <div class="users-actions" role="tablist">
          <For each={TABS}>
            {(tab) => (
              <button
                type="button"
                role="tab"
                aria-selected={activeTab() === tab.id}
                class={activeTab() === tab.id ? 'users-btn users-btn-secondary' : 'users-btn'}
                data-users-tab={tab.id}
                onClick={() => setActiveTab(tab.id)}
              >
                {t(tab.labelKey)}
              </button>
            )}
          </For>
        </div>
      </div>

      <Show when={activeTab() === 'users'}>
        <div data-users-panel="users">
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
        <Show when={tempPassword()}>
          {(pwd) => (
            <div class="users-card" data-temp-password="">
              <h3 class="users-dialog-title">{t('tempPasswordTitle')}</h3>
              <p class="users-dialog-intro">{t('tempPasswordOnce')}</p>
              <p><code>{pwd()}</code></p>
              <button
                type="button"
                class="users-btn users-btn-primary"
                data-users-action="temp-password-done"
                onClick={() => setTempPassword(null)}
              >
                {t('tempPasswordDone')}
              </button>
            </div>
          )}
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
                      const isSuspended = () => user.status === 'SUSPENDED';
                      const isDisabled = () => user.status === 'DISABLED';

                      return (
                        <tr data-user-id={user.userId}>
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

                              <Show when={isActive() || isSuspended()}>
                                <button
                                  type="button"
                                  class="users-btn"
                                  data-users-action="profile"
                                  onClick={() => openProfile(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('profileButton')}
                                </button>
                                <button
                                  type="button"
                                  class="users-btn"
                                  data-users-action="reset"
                                  onClick={() => handleResetPassword(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('resetPassword')}
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
                                  class="users-btn"
                                  data-users-action="suspend"
                                  onClick={() => handleSuspend(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('suspend')}
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

                              <Show when={isSuspended()}>
                                <button
                                  type="button"
                                  class="users-btn"
                                  data-users-action="resume"
                                  onClick={() => handleResume(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('resume')}
                                </button>
                              </Show>

                              <Show when={isDisabled()}>
                                <button
                                  type="button"
                                  class="users-btn"
                                  data-users-action="enable"
                                  onClick={() => handleEnable(user)}
                                  disabled={rowWriting()}
                                >
                                  {t('enable')}
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

      {/* Profile Dialog */}
      <Show when={profilingUser()}>
        {(user) => (
          <div class="users-dialog-backdrop">
            <div role="dialog" aria-modal="true" class="users-dialog">
              <h3 class="users-dialog-title">{t('profileTitle')}: {user().name}</h3>
              <Show when={dialogError()}>
                <div role="alert" class="users-form-error">{dialogError()}</div>
              </Show>
              <form class="users-dialog-form" data-users-form="profile" onSubmit={handleProfileSubmit}>
                <div class="users-field">
                  <label class="users-label" for="profile-employee-code">{t('profileEmployeeCode')}</label>
                  <input
                    id="profile-employee-code"
                    name="employeeCode"
                    class="users-input"
                    value={profileEmployeeCode()}
                    onInput={(e) => setProfileEmployeeCode(e.currentTarget.value)}
                  />
                </div>

                <div class="users-field">
                  <label class="users-label" for="profile-department">{t('profileDepartment')}</label>
                  <input
                    id="profile-department"
                    name="department"
                    class="users-input"
                    value={profileDepartment()}
                    onInput={(e) => setProfileDepartment(e.currentTarget.value)}
                  />
                </div>

                <div class="users-field">
                  <label class="users-label" for="profile-position">{t('profilePosition')}</label>
                  <input
                    id="profile-position"
                    name="position"
                    class="users-input"
                    value={profilePosition()}
                    onInput={(e) => setProfilePosition(e.currentTarget.value)}
                  />
                </div>

                <div class="users-field">
                  <label class="users-label" for="profile-tell-code">{t('profileTellCode')}</label>
                  <input
                    id="profile-tell-code"
                    name="tellCode"
                    class="users-input"
                    value={profileTellCode()}
                    onInput={(e) => setProfileTellCode(e.currentTarget.value)}
                  />
                </div>

                <div class="users-dialog-actions">
                  <button
                    type="button"
                    class="users-btn users-btn-secondary"
                    onClick={() => setProfilingUser(null)}
                  >
                    {t('close')}
                  </button>
                  <button
                    type="submit"
                    class="users-btn users-btn-primary"
                    data-users-action="profile-save"
                    disabled={dialogSubmitting()}
                  >
                    {t('profileSave')}
                  </button>
                </div>
              </form>
            </div>
          </div>
        )}
      </Show>
        </div>
      </Show>

      <For each={TABS.filter((tab) => tab.id !== 'users')}>
        {(tab) => {
          const Comp = resolveTabComponent(tab.file!);
          return (
            <Show when={activeTab() === tab.id}>
              <div data-users-panel={tab.id}>
                {Comp ? (
                  <Comp
                    shown={() => props.shown() && activeTab() === tab.id}
                    branch={props.branch}
                  />
                ) : (
                  <div class="users-card users-empty" data-users-tab-pending="">
                    {t('tabPending')}
                  </div>
                )}
              </div>
            </Show>
          );
        }}
      </For>
    </div>
  );
}
