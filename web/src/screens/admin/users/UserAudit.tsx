import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { UsersTabProps } from './UsersScreen';
import * as table from './i18n';
import './users.css';

const t = screenT('admin.users', table);

// moves to web/src/shared/api/generated/admin_user_audit.ts when F4a merges
interface AdminUserAuditRow {
  id: string;
  at: string; // RFC 3339
  actorId: string;
  actorName: string;
  subjectId: string;
  subjectName: string;
  action: string; // e.g. GRANT, REVOKE, DISABLE, SUSPEND, RESUME, ENABLE, PROFILE_SAVE, CREATE
  before: unknown | null; // jsonb; null on rows written before M
  after: unknown | null;
  ip: string | null;
}

export function UserAudit(props: UsersTabProps): JSX.Element {
  const api = new ApiClient();
  const [rows, setRows] = createSignal<AdminUserAuditRow[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal<boolean>(false);

  let auditSeq = 0;
  let activeBranch = '';

  const loadAudit = async () => {
    const seq = ++auditSeq;
    setLoading(true);
    setError(null);
    try {
      const res = await api.page<AdminUserAuditRow>('admin_user_audit', {
        branch: props.branch(),
        limit: 100,
      });
      if (seq !== auditSeq) return;
      setRows(res);
    } catch (err: unknown) {
      if (seq !== auditSeq) return;
      setError(wordedError(err));
      setRows([]);
    } finally {
      if (seq === auditSeq) {
        setLoading(false);
      }
    }
  };

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    if (activeBranch && activeBranch !== branch) {
      setRows([]);
      setError(null);
    }
    activeBranch = branch;
    loadAudit();
  });

  const formatWhen = (at: string): string => {
    try {
      return new Date(at).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
    } catch {
      return at;
    }
  };

  const renderJson = (val: unknown): JSX.Element => {
    if (val === null || val === undefined) return '—';
    return <code>{JSON.stringify(val)}</code>;
  };

  return (
    <div class="users-card">
      <Show when={error()}>
        <div role="alert" class="users-form-error">{error()}</div>
      </Show>
      <Show when={!error()}>
        <Show
          when={rows().length > 0}
          fallback={
            <div class="users-empty">
              {loading() ? t('loading') : t('auditEmpty')}
            </div>
          }
        >
          <div class="users-table-scroll">
            <table class="users-table">
              <thead>
                <tr>
                  <th class="users-th">{t('auditWhen')}</th>
                  <th class="users-th">{t('auditActor')}</th>
                  <th class="users-th">{t('auditSubject')}</th>
                  <th class="users-th">{t('auditAction')}</th>
                  <th class="users-th">{t('auditBefore')}</th>
                  <th class="users-th">{t('auditAfter')}</th>
                  <th class="users-th">{t('auditIp')}</th>
                </tr>
              </thead>
              <tbody>
                <For each={rows()}>
                  {(row) => (
                    <tr>
                      <td class="users-td">{formatWhen(row.at)}</td>
                      <td class="users-td">
                        <div><strong>{row.actorName || row.actorId}</strong></div>
                        <Show when={row.actorName && row.actorName !== row.actorId}>
                          <small class="users-intro">{row.actorId}</small>
                        </Show>
                      </td>
                      <td class="users-td">
                        <div><strong>{row.subjectName || row.subjectId}</strong></div>
                        <Show when={row.subjectName && row.subjectName !== row.subjectId}>
                          <small class="users-intro">{row.subjectId}</small>
                        </Show>
                      </td>
                      <td class="users-td">{row.action}</td>
                      <td class="users-td">{renderJson(row.before)}</td>
                      <td class="users-td">{renderJson(row.after)}</td>
                      <td class="users-td">{row.ip ?? '—'}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </div>
  );
}
