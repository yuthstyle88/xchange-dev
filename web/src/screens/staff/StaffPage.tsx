import { For, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import type { ApiClient } from '../../shared/api/client';
import type { StaffCard, StaffCtx } from './cards';
import { can } from './parts/can';
import { th, en } from './i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { createStaffContext } from './staff-context';
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
  const {
    me,
    floor,
    error: floorError,
    refresh,
  } = createStaffContext({
    api: props.api,
    signInUrl: '/login?next=%2Fstaff',
    navigate: props.navigate,
  });

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
                  <Show
                    when={
                      can(currentMe(), 'ShiftCloseCertify') ||
                      can(currentMe(), 'FinanceReconcile') ||
                      can(currentMe(), 'OverviewRead')
                    }
                  >
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
