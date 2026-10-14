import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { errorText, screenT } from '../../../shared/i18n';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { CustomerHoldCreateInput, CustomerHoldReleaseInput } from '../../../shared/api/generated/branch_day';
import type { StaffCustomerRow, StaffCustomersFilter } from '../../../shared/api/generated/staff_customers';
import type { StaffHoldRow } from '../../../shared/api/generated/staff_holds';
import type { StaffCtx } from '../cards';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { ReasonDialog } from '../parts/ReasonDialog';
import { th, en } from './i18n';

export interface HoldsCardProps {
  ctx: StaffCtx;
}

function toErrorText(e: unknown): string {
  const generic = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
  if (e instanceof ApiError) {
    const text = errorText(e.code, '');
    return text || generic;
  }
  if (e && typeof e === 'object' && 'code' in e && (e as { code: unknown }).code) {
    const code = String((e as { code: unknown }).code);
    const text = errorText(code, '');
    return text || generic;
  }
  if (e instanceof Error && e.message) {
    const text = errorText(e.message, '');
    return text || generic;
  }
  if (typeof e === 'string' && e) {
    const text = errorText(e, '');
    return text || generic;
  }
  return generic;
}

export function HoldsCard(props: HoldsCardProps): JSX.Element {
  const t = screenT('staff.holds', { th, en });
  const api = new ApiClient();

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const [holds, setHolds] = createSignal<StaffHoldRow[]>([]);
  const [searchKey, setSearchKey] = createSignal<string>('');
  const [customers, setCustomers] = createSignal<StaffCustomerRow[]>([]);
  const [selectedCustomer, setSelectedCustomer] = createSignal<StaffCustomerRow | null>(null);
  const [reason, setReason] = createSignal<string>('');

  const [releaseTarget, setReleaseTarget] = createSignal<StaffHoldRow | null>(null);

  const placeCmd = command<CustomerHoldCreateInput, { id: string }>(
    api,
    'POST',
    '/api/customer-holds',
  );

  const releaseCmdMap = new Map<string, { send(body: CustomerHoldReleaseInput): Promise<{ success?: boolean }> }>();

  function getReleaseCmd(id: string) {
    let c = releaseCmdMap.get(id);
    if (!c) {
      c = command<CustomerHoldReleaseInput, { success?: boolean }>(
        api,
        'POST',
        `/api/customer-holds/${id}/release`,
      );
      releaseCmdMap.set(id, c);
    }
    return c;
  }

  const loadHolds = async () => {
    try {
      const res = await api.view<StaffHoldRow[]>('staff_holds');
      if (res.data) {
        setHolds(res.data);
      }
    } catch (e: unknown) {
      setError(toErrorText(e));
    }
  };

  onMount(() => {
    void loadHolds();
  });

  const handleFind = async () => {
    const key = searchKey().trim();
    if (!key) {
      setCustomers([]);
      return;
    }
    try {
      setError(null);
      const res = await api.view<StaffCustomerRow[], StaffCustomersFilter>('staff_customers', {
        filter: { q: searchKey() },
      });
      if (res.data) {
        setCustomers(res.data);
      }
    } catch (e: unknown) {
      setCustomers([]);
      setError(toErrorText(e));
    }
  };

  const handlePlace = async () => {
    const cust = selectedCustomer();
    if (!cust) {
      return;
    }
    try {
      await placeCmd.send({
        customerId: cust.id,
        reason: reason(),
      } as CustomerHoldCreateInput);
      setNotice(t('saved'));
      setError(null);
      await props.ctx.refresh();
      await loadHolds();
    } catch (e) {
      setNotice(null);
      setError(toErrorText(e));
    }
  };

  const handleReleaseConfirm = async (note: string) => {
    const target = releaseTarget();
    if (!target) return;
    try {
      const cmd = getReleaseCmd(target.id);
      await cmd.send({
        id: target.id,
        note,
      } as CustomerHoldReleaseInput);
      setNotice(t('saved'));
      setError(null);
      setReleaseTarget(null);
      await props.ctx.refresh();
      await loadHolds();
    } catch (e) {
      setNotice(null);
      setError(toErrorText(e));
    }
  };

  const mayPlace = () => can(props.ctx.me, 'CustomerHoldPlace');
  const mayRelease = () => can(props.ctx.me, 'VerificationReview');

  return (
    <div data-card="holds">
      <Show when={notice()}>
        <div data-testid="holds-notice">{notice()}</div>
      </Show>
      <Show when={error()}>
        <div data-testid="holds-error">{error()}</div>
      </Show>

      <Show when={mayPlace()}>
        <form
          data-testid="holds-form"
          onSubmit={(e) => {
            e.preventDefault();
            void handlePlace();
          }}
        >
          <input
            type="text"
            data-testid="holds-key"
            value={searchKey()}
            onInput={(e) => setSearchKey(e.currentTarget.value)}
          />
          <button type="button" data-testid="holds-find" onClick={handleFind}>
            {t('find')}
          </button>

          <For each={customers()}>
            {(cust) => (
              <div
                data-customer-id={cust.id}
                onClick={() => setSelectedCustomer(cust)}
              >
                {cust.displayName}
              </div>
            )}
          </For>

          <input
            type="text"
            data-testid="holds-reason"
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
          <button type="button" data-testid="holds-place" onClick={handlePlace}>
            {t('place')}
          </button>
        </form>
      </Show>

      <div>
        <For each={holds()}>
          {(hold) => (
            <div data-hold-id={hold.id}>
              <span>{hold.customerName ?? hold.customerId}</span>
              <span>{hold.reason}</span>
              <span>{hold.status}</span>
              <Show when={hold.status === 'OPEN' && mayRelease()}>
                <button
                  type="button"
                  data-testid="holds-release"
                  onClick={() => setReleaseTarget(hold)}
                >
                  {t('release')}
                </button>
              </Show>
            </div>
          )}
        </For>
      </div>

      <ReasonDialog
        open={!!releaseTarget()}
        title={t('releaseTitle')}
        onConfirm={handleReleaseConfirm}
        onCancel={() => setReleaseTarget(null)}
      />
    </div>
  );
}
