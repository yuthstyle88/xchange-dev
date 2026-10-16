import { batch, createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../shared/api/client';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { StaffFloorView } from '../../shared/api/generated/staff_floor';
import type { StaffCard, StaffCtx } from './cards';
import { can } from './parts/can';
import { th, en } from './i18n';
import { AppShell } from '../../shared/ui/AppShell';
import './staff.css';

export interface StaffPageProps {
  api: ApiClient;
  cards: readonly StaffCard[];
  navigate: (url: string) => void;
}

function formatBangkokTime(iso: string): string {
  const d = new Date(iso);
  return new Intl.DateTimeFormat('en-GB', {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    timeZone: 'Asia/Bangkok',
  }).format(d);
}

export function StaffPage(props: StaffPageProps): JSX.Element {
  const t = screenT('staff', { th, en });
  const [me, setMe] = createSignal<StaffMeView | undefined>(undefined);
  const [floor, setFloor] = createSignal<StaffFloorView | undefined>(undefined);
  const [floorError, setFloorError] = createSignal<string | undefined>(undefined);

  let seq = 0;

  const refresh = async (): Promise<void> => {
    const mine = ++seq;
    let meData: StaffMeView | undefined;
    try {
      const meRes = await props.api.view<StaffMeView>('staff_me');
      if (mine !== seq) return;
      meData = meRes.data;
    } catch (err: unknown) {
      if (mine !== seq) return;
      if (
        (typeof err === 'object' && err !== null && 'status' in err && (err as { status: number }).status === 401) ||
        (err instanceof ApiError && err.status === 401)
      ) {
        props.navigate('/login?next=%2Fstaff');
        return;
      }
      setFloorError(wordedError(err));
      return;
    }

    let floorData: StaffFloorView | undefined;
    let floorErr: string | undefined;
    try {
      const floorRes = await props.api.view<StaffFloorView>('staff_floor');
      if (mine !== seq) return;
      floorData = floorRes.data;
    } catch (err: unknown) {
      if (mine !== seq) return;
      floorErr = wordedError(err);
    }

    batch(() => {
      if (meData) {
        setMe(meData);
      }
      if (floorData !== undefined) {
        setFloor(floorData);
      }
      setFloorError(floorErr);
    });
  };

  onMount(() => {
    void refresh();
  });

  const hoursText = () => {
    const h = floor()?.hours;
    if (h && typeof h.open === 'string' && typeof h.close === 'string') {
      const open = formatBangkokTime(h.open);
      const close = formatBangkokTime(h.close);
      return t('hoursWithTimes', { open, close });
    }
    return t('hoursWithoutTimes');
  };

  const sortedCards = () =>
    [...props.cards].sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

  return (
    <Show
      when={me()}
      fallback={
        <Show when={floorError()}>
          {(err) => (
            <div class="xc-staff-container">
              <div role="alert" data-testid="staff-error">
                {err()}
              </div>
              <button
                type="button"
                data-testid="staff-refresh"
                class="xc-staff-refresh-btn"
                onClick={() => {
                  void refresh();
                }}
              >
                {t('refresh')}
              </button>
            </div>
          )}
        </Show>
      }
    >
      {(currentMe) => {
        const ctx = (): StaffCtx => ({
          me: currentMe(),
          floor: floor(),
          refresh,
        });

        return (
          <AppShell title={t('title')}>
            <div class="xc-staff-container">
              <header class="xc-staff-header">
                <div class="xc-staff-header-info">
                  <p data-testid="staff-hours" class="xc-staff-hours">
                    {hoursText()}
                  </p>
                </div>
                <div class="xc-staff-header-actions">
                  <button
                    type="button"
                    data-testid="staff-refresh"
                    class="xc-staff-refresh-btn"
                    onClick={() => {
                      void refresh();
                    }}
                  >
                    {t('refresh')}
                  </button>
                  <Show when={can(currentMe(), 'FinanceReconcile') || can(currentMe(), 'BranchDayOperate')}>
                    <a href="/staff/report" class="xc-staff-report-link">
                      {t('reportLink')}
                    </a>
                  </Show>
                </div>
              </header>

              <Show when={floorError()}>
                {(err) => (
                  <div role="alert" data-testid="staff-error">
                    {err()}
                  </div>
                )}
              </Show>

              <section class="xc-staff-cards">
                <For each={sortedCards()}>
                  {(card) => {
                    const Comp = card.Component;
                    return (
                      <Show when={card.visible(ctx())}>
                        <div data-card={card.id} class="xc-staff-card-wrapper">
                          <Comp ctx={ctx()} />
                        </div>
                      </Show>
                    );
                  }}
                </For>
              </section>
            </div>
          </AppShell>
        );
      }}
    </Show>
  );
}
