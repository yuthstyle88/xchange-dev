import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  AdminGroupAddInput,
  AdminGroupDeleteInput,
  AdminGroupOutput,
  AdminGroupSaveInput,
} from '../../../shared/api/generated/admin_groups';
import type {
  AdminPermissionMatrixData,
  MatrixGroup,
} from '../../../shared/api/generated/admin_permission_matrix';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import {
  ADMIN_BASE,
  ApprovalsEditor,
  ApprovalsOverview,
  isApprovalKind,
} from './ApprovalsSection';
import type { UsersTabProps } from './UsersScreen';
import * as table from './i18n';
import './users.css';

const t = screenT('admin.users', table);

export function PermissionMatrix(props: UsersTabProps): JSX.Element {
  const api = new ApiClient();
  const addWrite = keyedWrite<AdminGroupAddInput, AdminGroupOutput>(api, 'POST', '/api/admin/groups/add');
  const saveWrite = keyedWrite<AdminGroupSaveInput, AdminGroupOutput>(api, 'POST', '/api/admin/groups/save');
  const deleteWrite = keyedWrite<AdminGroupDeleteInput, AdminGroupOutput>(api, 'POST', '/api/admin/groups/delete');

  const [data, setData] = createSignal<AdminPermissionMatrixData | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal<boolean>(false);

  const [editingCode, setEditingCode] = createSignal<string | null>(null);
  const [editPerms, setEditPerms] = createSignal<Set<string>>(new Set());

  const [adding, setAdding] = createSignal<boolean>(false);
  const [addCode, setAddCode] = createSignal<string>('');
  const [addName, setAddName] = createSignal<string>('');
  const [addBase, setAddBase] = createSignal<string>('');
  const [addPerms, setAddPerms] = createSignal<Set<string>>(new Set());

  let loadSeq = 0;
  let activeBranch = '';

  const matrixError = (err: unknown): string => {
    if (err instanceof ApiError) {
      switch (err.code) {
        case 'guardrail:1':
          return t('matrixGuardrail_1');
        case 'guardrail:2':
          return t('matrixGuardrail_2');
        case 'guardrail:3':
          return t('matrixGuardrail_3');
        case 'guardrail:4':
          return t('matrixGuardrail_4');
        case 'guardrail:5':
          return t('matrixGuardrail_5');
        case 'guardrail:6':
          return t('matrixGuardrail_6');
        case 'builtIn':
          return t('matrixBuiltIn');
        case 'inUse':
          return t('matrixInUse');
      }
    }
    return wordedError(err);
  };

  const loadMatrix = async () => {
    const seq = ++loadSeq;
    setLoading(true);
    try {
      const res = await api.view<AdminPermissionMatrixData>('admin_permission_matrix', {
        branch: props.branch(),
      });
      if (seq !== loadSeq) return;
      if (res.data) {
        setData(res.data);
      }
    } catch (err: unknown) {
      if (seq !== loadSeq) return;
      setError(matrixError(err));
      setData(null);
    } finally {
      if (seq === loadSeq) {
        setLoading(false);
      }
    }
  };

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    if (activeBranch && activeBranch !== branch) {
      setData(null);
      setError(null);
      setEditingCode(null);
      setAdding(false);
    }
    activeBranch = branch;
    void loadMatrix();
  });

  const anyEditable = () => (data()?.groups ?? []).some((g) => g.editable);
  const editablePermissions = () =>
    (data()?.permissions ?? []).filter((p) => p.editable && p.module !== null);
  const baseOptions = () =>
    (data()?.groups ?? []).filter((g) => g.builtIn && g.code !== 'EXECUTIVE');
  const editingGroup = () =>
    (data()?.groups ?? []).find((g) => g.code === editingCode());

  const orderedPermissions = (selected: Set<string>, includeApprovals = false): string[] =>
    (data()?.permissions ?? [])
      .filter((p) => (p.editable && p.module !== null) || (includeApprovals && isApprovalKind(p.name)))
      .map((p) => p.name)
      .filter((name) => selected.has(name));

  const levelText = (level: string | undefined): string => {
    switch (level) {
      case 'VIEW':
        return t('matrixLevelView');
      case 'CREATE':
        return t('matrixLevelCreate');
      case 'FULL':
        return t('matrixLevelFull');
      default:
        return t('matrixLevelNone');
    }
  };

  const moduleLabel = (mod: string): string => {
    const key = `matrixModule_${mod}` as keyof typeof table.th;
    return key in table.th ? t(key) : mod;
  };

  const setPermIn = (
    current: Set<string>,
    setter: (next: Set<string>) => void,
    name: string,
    checked: boolean,
  ) => {
    const next = new Set(current);
    if (checked) next.add(name);
    else next.delete(name);
    setter(next);
  };

  const openEdit = (group: MatrixGroup) => {
    setError(null);
    setAdding(false);
    setEditingCode(group.code);
    const allowKinds = group.base !== ADMIN_BASE;
    const allowed = new Set(editablePermissions().map((p) => p.name));
    setEditPerms(
      new Set(
        group.permissions.filter((name) => allowed.has(name) || (allowKinds && isApprovalKind(name))),
      ),
    );
  };

  const openAdd = () => {
    setError(null);
    setEditingCode(null);
    setAddCode('');
    setAddName('');
    setAddBase(baseOptions()[0]?.code ?? '');
    setAddPerms(new Set<string>());
    setAdding(true);
  };

  const handleSave = async () => {
    const code = editingCode();
    if (!code) return;
    setError(null);
    const allowKinds = editingGroup()?.base !== ADMIN_BASE;
    try {
      await saveWrite.send({
        code,
        permissions: orderedPermissions(editPerms(), allowKinds),
      });
      setEditingCode(null);
      await loadMatrix();
    } catch (err: unknown) {
      setError(matrixError(err));
    }
  };

  const handleAdd = async () => {
    setError(null);
    try {
      await addWrite.send({
        code: addCode().trim(),
        name: addName().trim(),
        base: addBase(),
        permissions: orderedPermissions(addPerms()),
      });
      setAdding(false);
      await loadMatrix();
    } catch (err: unknown) {
      setError(matrixError(err));
    }
  };

  const handleDelete = async (code: string) => {
    if (!confirm(t('matrixDeleteConfirm'))) return;
    setError(null);
    try {
      await deleteWrite.send({ code });
      if (editingCode() === code) {
        setEditingCode(null);
      }
      await loadMatrix();
    } catch (err: unknown) {
      setError(matrixError(err));
    }
  };

  return (
    <div class="users-card">
      <div class="users-toolbar">
        <div>
          <h3 class="users-dialog-title">{t('matrixTitle')}</h3>
          <Show when={data() && !anyEditable()}>
            <p class="users-intro" data-matrix-readonly="">
              {t('matrixReadOnly')}
            </p>
          </Show>
        </div>
        <Show when={anyEditable()}>
          <div class="users-actions">
            <button
              type="button"
              class="users-btn users-btn-primary"
              data-matrix-add-open=""
              onClick={openAdd}
            >
              {t('matrixAddOpen')}
            </button>
          </div>
        </Show>
      </div>

      <Show when={error()}>
        <div role="alert" class="users-form-error" data-matrix-error="">
          {error()}
        </div>
      </Show>

      <Show
        when={data() && data()!.groups.length > 0}
        fallback={
          <div class="users-empty">
            {loading() ? t('loading') : t('matrixEmpty')}
          </div>
        }
      >
        <div class="users-table-scroll">
          <table class="users-table">
            <thead>
              <tr>
                <th class="users-th">{t('matrixRole')}</th>
                <For each={data()!.modules}>
                  {(mod) => <th class="users-th">{moduleLabel(mod)}</th>}
                </For>
                <Show when={anyEditable()}>
                  <th class="users-th">{t('colActions')}</th>
                </Show>
              </tr>
            </thead>
            <tbody>
              <For each={data()!.groups}>
                {(group) => (
                  <tr data-matrix-group={group.code}>
                    <td class="users-td">
                      <div>
                        <strong>{group.name}</strong>
                      </div>
                      <Show when={group.name !== group.code}>
                        <small class="users-intro">{group.code}</small>
                      </Show>
                    </td>
                    <For each={data()!.modules}>
                      {(mod) => (
                        <td
                          class="users-td"
                          data-matrix-level={`${group.code}:${mod}`}
                        >
                          {levelText(group.levels[mod])}
                        </td>
                      )}
                    </For>
                    <Show when={anyEditable()}>
                      <td class="users-td">
                        <div class="users-row-actions">
                          <Show when={group.editable}>
                            <button
                              type="button"
                              class="users-btn users-btn-secondary"
                              data-matrix-edit={group.code}
                              onClick={() => openEdit(group)}
                            >
                              {t('matrixEdit')}
                            </button>
                          </Show>
                          <Show when={group.editable && !group.builtIn}>
                            <button
                              type="button"
                              class="users-btn users-btn-disable"
                              data-matrix-delete={group.code}
                              onClick={() => void handleDelete(group.code)}
                            >
                              {t('matrixDelete')}
                            </button>
                          </Show>
                        </div>
                      </td>
                    </Show>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
        <ApprovalsOverview groups={data()!.groups} />
      </Show>

      <Show when={editingCode()}>
        <div class="users-matrix-panel">
          <h4 class="users-dialog-title">
            {t('matrixEditTitle')}: {editingCode()}
          </h4>
          <div class="users-dialog-form">
            <div class="users-matrix-perms">
              <For each={editablePermissions()}>
                {(perm) => (
                  <label class="users-matrix-perm">
                    <input
                      type="checkbox"
                      data-matrix-perm={perm.name}
                      checked={editPerms().has(perm.name)}
                      onChange={(e) =>
                        setPermIn(editPerms(), setEditPerms, perm.name, e.currentTarget.checked)
                      }
                    />
                    <span>{perm.name}</span>
                  </label>
                )}
              </For>
            </div>
            <ApprovalsEditor
              base={editingGroup()?.base ?? ''}
              selected={editPerms()}
              onToggle={(kind, on) => setPermIn(editPerms(), setEditPerms, kind, on)}
            />
            <div class="users-dialog-actions">
              <button
                type="button"
                class="users-btn users-btn-secondary"
                onClick={() => setEditingCode(null)}
              >
                {t('matrixCancel')}
              </button>
              <button
                type="button"
                class="users-btn users-btn-primary"
                data-matrix-save=""
                onClick={() => void handleSave()}
              >
                {t('matrixSave')}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <Show when={adding()}>
        <div class="users-matrix-panel" data-matrix-add-form="">
          <h4 class="users-dialog-title">{t('matrixAddTitle')}</h4>
          <div class="users-dialog-form">
            <div class="users-field">
              <label class="users-label">{t('matrixGroupCode')}</label>
              <input
                type="text"
                class="users-input"
                data-matrix-add-code=""
                value={addCode()}
                onInput={(e) => setAddCode(e.currentTarget.value)}
              />
            </div>
            <div class="users-field">
              <label class="users-label">{t('matrixGroupName')}</label>
              <input
                type="text"
                class="users-input"
                data-matrix-add-name=""
                value={addName()}
                onInput={(e) => setAddName(e.currentTarget.value)}
              />
            </div>
            <div class="users-field">
              <label class="users-label">{t('matrixGroupBase')}</label>
              <select
                class="users-select"
                data-matrix-add-base=""
                value={addBase()}
                onInput={(e) => setAddBase(e.currentTarget.value)}
                onChange={(e) => setAddBase(e.currentTarget.value)}
              >
                <For each={baseOptions()}>
                  {(base) => <option value={base.code}>{base.name}</option>}
                </For>
              </select>
            </div>
            <div class="users-field">
              <span class="users-label">{t('matrixPermissions')}</span>
              <div class="users-matrix-perms">
                <For each={editablePermissions()}>
                  {(perm) => (
                    <label class="users-matrix-perm">
                      <input
                        type="checkbox"
                        data-matrix-perm={perm.name}
                        checked={addPerms().has(perm.name)}
                        onChange={(e) =>
                          setPermIn(addPerms(), setAddPerms, perm.name, e.currentTarget.checked)
                        }
                      />
                      <span>{perm.name}</span>
                    </label>
                  )}
                </For>
              </div>
            </div>
            <div class="users-dialog-actions">
              <button
                type="button"
                class="users-btn users-btn-secondary"
                onClick={() => setAdding(false)}
              >
                {t('matrixCancel')}
              </button>
              <button
                type="button"
                class="users-btn users-btn-primary"
                data-matrix-add-submit=""
                onClick={() => void handleAdd()}
              >
                {t('matrixAddSubmit')}
              </button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}

