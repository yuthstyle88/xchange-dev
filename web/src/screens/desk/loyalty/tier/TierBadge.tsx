// B03 the customer's tier (FE-desk skeleton stub, #564; view `customer_tier?customer=`,
// api/src/presentation/http/v3/views/customer_tier.rs, type shared/api/generated/customer_tier.ts). Filled by its own ticket.
import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../../shared/api/client';
import type { CustomerTierData } from '../../../../shared/api/generated/customer_tier';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import { en, th } from './i18n';

export interface TierBadgeProps {
  api: ApiClient;
  branch: string;
  customerId: string;
  /** Changes after a phone is verified; the badge reads the view again. */
  refreshKey: number;
}

export function TierBadge(props: TierBadgeProps): JSX.Element {
  const t = screenT('desk.loyalty.tier', { th, en });
  const [tier, setTier] = createSignal<string | null | undefined>(undefined);
  const [error, setError] = createSignal<string | undefined>(undefined);

  let latestReq = 0;

  createEffect(() => {
    const customerId = props.customerId;
    const branch = props.branch;
    void props.refreshKey;

    if (!customerId) {
      setTier(undefined);
      setError(undefined);
      return;
    }

    const reqId = ++latestReq;

    props.api
      .view<CustomerTierData>('customer_tier', {
        branch,
        customer: customerId,
        filter: { customer: customerId },
      } as any)
      .then((res) => {
        if (reqId !== latestReq) return;
        setError(undefined);
        setTier(res.data?.tier ?? null);
      })
      .catch((err: unknown) => {
        if (reqId !== latestReq) return;
        setTier(undefined);
        setError(wordedError(err));
      });
  });

  return (
    <div>
      <Show when={error()}>
        {(msg) => <p class="desk-notice">{msg()}</p>}
      </Show>
      <Show when={!error()}>
        <Show when={tier() !== undefined} fallback={<p>{t('loading')}</p>}>
          <Show when={tier() !== null} fallback={<p>{t('noTier')}</p>}>
            <p>
              <span>{t('tier')}</span>: <strong>{tier()}</strong>
            </p>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
