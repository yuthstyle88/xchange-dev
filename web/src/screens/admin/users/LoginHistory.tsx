import { createSignal, createEffect, Show, For, type JSX } from 'solid-js';
import { screenT, locale, t as sharedT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
import type { UsersTabProps } from './UsersScreen';
import * as table from './i18n';

const t = screenT('admin.users', table);

// moves to web/src/shared/api/generated/admin_login_history.ts when F4b merges
type LoginResult =
  | 'OK'
  | 'BAD_PASSWORD'
  | 'UNKNOWN_USER'
  | 'NOT_ACTIVE'
  | 'TEMP_EXPIRED'
  | 'RATE_LIMITED'
  | 'UNAVAILABLE';

interface AdminLoginHistoryRow {
  id: string;
  at: string; // RFC 3339
  userId: string | null; // null when the identifier matched no user
  userName: string | null;
  ip: string | null;
  result: LoginResult;
}

function formatResult(result: string): string {
  switch (result) {
    case 'OK':
      return t('loginsResultOk');
    case 'BAD_PASSWORD':
      return t('loginsResultBadPassword');
    case 'UNKNOWN_USER':
      return t('loginsResultUnknownUser');
    case 'NOT_ACTIVE':
      return t('loginsResultNotActive');
    case 'TEMP_EXPIRED':
      return t('loginsResultTempExpired');
    case 'RATE_LIMITED':
      return t('loginsResultRateLimited');
    case 'UNAVAILABLE':
      return sharedT('error.serverError');
    default:
      return result;
  }
}

function formatWhen(at: string): string {
  try {
    return new Date(at).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
  } catch {
    return at;
  }
}

export function LoginHistory(props: UsersTabProps): JSX.Element {
  const api = new ApiClient();
  const [rows, setRows] = createSignal<AdminLoginHistoryRow[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);

  let seq = 0;

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    const currentSeq = ++seq;

    api
      .page<AdminLoginHistoryRow>('admin_login_history', { branch, limit: 100 })
      .then((data) => {
        if (currentSeq !== seq) return;
        setError(null);
        setRows(data);
        setLoaded(true);
      })
      .catch((err) => {
        if (currentSeq !== seq) return;
        setError(wordedError(err));
        setRows([]);
        setLoaded(true);
      });
  });

  return (
    <div class="users-tab-panel">
      <Show when={error()}>
        <div class="users-card users-empty" data-users-error="">
          {error()}
        </div>
      </Show>
      <Show when={!error() && loaded() && rows().length === 0}>
        <div class="users-card users-empty" data-users-empty="">
          {t('loginsEmpty')}
        </div>
      </Show>
      <Show when={!error() && rows().length > 0}>
        <div class="users-card users-table-scroll">
          <table class="users-table">
            <thead>
              <tr>
                <th class="users-th">{t('loginsWhen')}</th>
                <th class="users-th">{t('loginsUser')}</th>
                <th class="users-th">{t('loginsResult')}</th>
                <th class="users-th">{t('loginsIp')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row) => (
                  <tr>
                    <td class="users-td">{formatWhen(row.at)}</td>
                    <td class="users-td">{row.userName ?? '—'}</td>
                    <td class="users-td">{formatResult(row.result)}</td>
                    <td class="users-td">{row.ip ?? '—'}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
