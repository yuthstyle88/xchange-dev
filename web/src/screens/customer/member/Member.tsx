import { Show, createSignal, onMount, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import type { MyMembershipData } from '../../../shared/api/generated/my_membership';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import './member.css';

const t = screenT('customer.member', table);

// stopgap (#1860): ApiClient.view reads only the {version, changed, data} envelope while LY-5 my_membership answers a bare MyMembershipData object; keep local error parsing until #1860 adds a bare-view read to ApiClient.
async function parseErrorResponse(res: Response): Promise<never> {
  const text = await res.text();
  try {
    const json = JSON.parse(text) as {
      error?: string | { code?: string; field?: string; reason?: string; message?: string };
    };
    if (json && typeof json === 'object' && json.error) {
      if (typeof json.error === 'string') {
        throw new ApiError({
          code: 'refused',
          message: json.error,
          status: res.status,
        });
      }
      throw new ApiError({
        code: json.error.code ?? 'unknown',
        field: json.error.field,
        reason: typeof json.error.reason === 'string' ? json.error.reason : undefined,
        message: json.error.message ?? text,
        status: res.status,
      });
    }
  } catch (e) {
    if (e instanceof ApiError) throw e;
  }
  throw new ApiError({
    code: 'httpError',
    message: text || res.statusText,
    status: res.status,
  });
}

// stopgap (#1860): return the bare MyMembershipData payload directly until #1860 adds a bare-view read to ApiClient.
function normalizeMembershipData(payload: MyMembershipData): MyMembershipData {
  return payload;
}

export function MemberScreen(): JSX.Element {
  const [loading, setLoading] = createSignal(true);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [data, setData] = createSignal<MyMembershipData | null>(null);

  const loadData = async () => {
    setLoading(true);
    setErrorMsg(null);
    try {
      // stopgap (#1860): raw fetch because ApiClient.view expects {version, changed, data} whereas LY-5 /api/v3/views/my_membership returns a bare MyMembershipData object.
      const res = await fetch('/api/v3/views/my_membership', {
        method: 'GET',
        credentials: 'same-origin',
      });
      if (!res.ok) {
        await parseErrorResponse(res);
      }
      const raw = (await res.json()) as MyMembershipData;
      setData(normalizeMembershipData(raw));
    } catch (err) {
      const message =
        err instanceof ApiError && err.code === 'outOfScope' ? t('outOfScope') : wordedError(err);
      setErrorMsg(message);
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void loadData();
  });

  const hasAnyPoints = (m: MyMembershipData): boolean =>
    Boolean(
      (m.points && (m.points.available > 0 || m.points.pending > 0)) ||
        (m.nextExpiry && m.nextExpiry.points > 0),
    );

  const shouldShowPoints = (m: MyMembershipData): boolean =>
    m.tier !== null || m.pointsUnavailable || hasAnyPoints(m);

  return (
    <div class="xc-member-screen">
      <header class="xc-member-header">
        <h1 class="xc-member-title">{t('title')}</h1>
      </header>

      <Show when={loading()}>
        <div class="xc-member-loading" role="status">
          {t('loading')}
        </div>
      </Show>

      <Show when={!loading() && errorMsg()}>
        {(msg) => (
          <div class="xc-member-error" role="alert">
            <p class="xc-member-error-message">{msg()}</p>
            <button type="button" class="xc-member-retry-btn" onClick={loadData}>
              {t('retry')}
            </button>
          </div>
        )}
      </Show>

      <Show when={!loading() && !errorMsg() && data()}>
        {(member) => (
          <div class="xc-member-content">
            <Show
              when={member().tier !== null}
              fallback={
                <div class="xc-member-not-member">
                  <h2 class="xc-member-not-member-title">{t('notMember')}</h2>
                  <Show when={!hasAnyPoints(member())}>
                    <p class="xc-member-how-to-join">{t('howToJoin')}</p>
                  </Show>
                </div>
              }
            >
              <div class="xc-member-tier-card">
                <span class="xc-member-tier-label">{t('tier')}</span>
                <span class="xc-member-tier-value">{member().tier}</span>
                <Show when={member().effectiveUntil}>
                  {(until) => (
                    <p class="xc-member-effective-until">
                      {t('effectiveUntil', { date: until() })}
                    </p>
                  )}
                </Show>
              </div>
            </Show>

            <Show when={shouldShowPoints(member())}>
              <div class="xc-member-points">
                <Show
                  when={!member().pointsUnavailable && member().points}
                  fallback={
                    <p class="xc-member-points-unavailable">{t('pointsUnavailable')}</p>
                  }
                >
                  {(pts) => (
                    <div class="xc-member-points-grid">
                      <div class="xc-member-points-available">
                        <span class="xc-member-points-label">{t('pointsAvailable')}</span>
                        <span class="xc-member-points-value">{String(pts().available)}</span>
                      </div>
                      <div class="xc-member-points-pending">
                        <span class="xc-member-points-label">{t('pointsPending')}</span>
                        <span class="xc-member-points-value">{String(pts().pending)}</span>
                      </div>
                    </div>
                  )}
                </Show>
              </div>
            </Show>

            <Show when={!member().pointsUnavailable && member().nextExpiry}>
              {(expiry) => (
                <div class="xc-member-expiry">
                  {t('nextExpiry', {
                    points: String(expiry().points),
                    date: expiry().at,
                  })}
                </div>
              )}
            </Show>

            <Show when={member().progress}>
              {(prog) => (
                <div class="xc-member-progress">
                  <h2 class="xc-member-progress-title">{t('progressTitle')}</h2>
                  <Show
                    when={prog().next !== null}
                    fallback={<p class="xc-member-progress-top">{t('topTier')}</p>}
                  >
                    <div class="xc-member-progress-next">
                      {t('progressText', {
                        current: prog().current,
                        next: prog().next ?? '',
                        basis: prog().basis,
                      })}
                    </div>
                  </Show>
                </div>
              )}
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
