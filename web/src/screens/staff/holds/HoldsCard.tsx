import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
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

export function HoldsCard(props: HoldsCardProps): JSX.Element {
  const t = screenT('staff.holds', { th, en });
  const api = new ApiClient();

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<boolean>(false);

  const [holds, setHolds] = createSignal<StaffHoldRow[]>([]);
  const [searchKey, setSearchKey] = createSignal<string>('');
  const [customers, setCustomers] = createSignal<StaffCustomerRow[]>([]);
  const [selectedCustomer, setSelectedCustomer] = createSignal<StaffCustomerRow | null>(null);
  const [reason, setReason] = createSignal<string>('');
  let findSeq = 0;

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
      setError(wordedError(e));
    }
  };

  onMount(() => {
    void loadHolds();
  });

  const handleFind = async () => {
    const seq = ++findSeq;
    setSelectedCustomer(null);
    setCustomers([]);
    const key = searchKey().trim();
    if (!key) {
      setSelectedCustomer(null);
      setCustomers([]);
      return;
    }
    try {
      setError(null);
      const res = await api.view<StaffCustomerRow[], StaffCustomersFilter>('staff_customers', {
        filter: { q: searchKey() },
      });
      if (seq !== findSeq) return;
      setSelectedCustomer(null);
      setCustomers(res.data ?? []);
    } catch (e: unknown) {
      if (seq !== findSeq) return;
      setSelectedCustomer(null);
      setCustomers([]);
      setError(wordedError(e));
    }
  };

  const handlePlace = async () => {
    if (busy()) return;
    const cust = selectedCustomer();
    if (!cust) {
      setNotice(null);
      setError(t('pickCustomer'));
      return;
    }
    setBusy(true);
    try {
      await placeCmd.send({
        customerId: cust.id,
        reason: reason(),
      } as CustomerHoldCreateInput);
      setSelectedCustomer(null);
      setReason('');
      setNotice(t('saved'));
      setError(null);
      await props.ctx.refresh();
      await loadHolds();
    } catch (e) {
      setNotice(null);
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleReleaseConfirm = async (note: string) => {
    if (busy()) return;
    const target = releaseTarget();
    if (!target) return;
    setBusy(true);
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
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  const mayPlace = () => can(props.ctx.me, 'CustomerHoldPlace');
  const mayRelease = () => can(props.ctx.me, 'VerificationReview');

  return (
    <div>
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
                aria-pressed={selectedCustomer()?.id === cust.id ? 'true' : 'false'}
                data-selected={selectedCustomer()?.id === cust.id ? 'true' : undefined}
                style={selectedCustomer()?.id === cust.id ? { 'font-weight': '600' } : undefined}
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
          <button
            type="button"
            data-testid="holds-place"
            disabled={busy()}
            onClick={handlePlace}
          >
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
