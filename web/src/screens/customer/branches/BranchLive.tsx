// C08: one branch card's live status — waiting, counters open, wait estimate (branch_service_status, public)
// and today's free booking slots (booking_slots, public) (design customer-app.md §4.5).
import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import type { BookingSlotsData } from '../../../shared/api/generated/booking_slots';
import type { BranchServiceStatusData } from '../../../shared/api/generated/branch_service_status';
import { screenT } from '../../../shared/i18n';
import { th, en, zh } from './i18n';

export interface BranchLiveProps {
  /** The branch's public code, which is its id. */
  branch: string;
  /** `PublicBranchDto.bookingEnabled`: no slots are read for a branch that takes no bookings. */
  bookingEnabled: boolean;
  /** The screen's fetch, so tests mock at the fetch level. */
  fetch: Fetch;
}

export function BranchLive(props: BranchLiveProps): JSX.Element {
  const t = screenT('customer.branches', { th, en, zh });
  const [status, setStatus] = createSignal<BranchServiceStatusData | null>(null);
  const [freeSlots, setFreeSlots] = createSignal<number | null>(null);

  onMount(() => {
    const api = new ApiClient(props.fetch);
    const branch = props.branch;

    void api
      .view<BranchServiceStatusData>('branch_service_status', { branch })
      .then((res) => {
        if (res.data) setStatus(res.data);
      })
      .catch(() => {});

    if (props.bookingEnabled) {
      void api
        .view<BookingSlotsData>('booking_slots', { branch })
        .then((res) => {
          if (res.data) {
            const now = Date.now();
            const count = res.data.slots.filter((s) => s.available > 0 && Date.parse(s.start) > now).length;
            setFreeSlots(count);
          }
        })
        .catch(() => {});
    }
  });

  return (
    <Show when={status() !== null || freeSlots() !== null}>
      <div class="xc-branch-live">
        <Show when={status()}>
          {(s) => (
            <>
              <span data-testid="live-waiting">
                {t('liveWaiting', { count: s().waiting.walkIn + s().waiting.booking })}
              </span>
              <span data-testid="live-counters">{t('liveCounters', { count: s().activeCounters })}</span>
              <Show when={s().estimateLowSeconds !== null && s().estimateHighSeconds !== null}>
                <span data-testid="live-estimate">
                  {t('liveEstimate', {
                    low: Math.ceil(s().estimateLowSeconds! / 60),
                    high: Math.ceil(s().estimateHighSeconds! / 60),
                  })}
                </span>
              </Show>
            </>
          )}
        </Show>
        <Show when={freeSlots() !== null}>
          <span data-testid="live-free-slots">{t('liveFreeSlots', { count: freeSlots()! })}</span>
        </Show>
      </div>
    </Show>
  );
}
