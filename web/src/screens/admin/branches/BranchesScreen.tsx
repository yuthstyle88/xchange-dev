import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type { AdminBranchProfileData } from '../../../shared/api/generated/admin_branches_profile';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './branches.css';

const t = screenT('admin.branches', table);

const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
type DayKey = (typeof DAYS)[number];

const DAY_LABELS: Record<DayKey, () => string> = {
  mon: () => t('dayMon'),
  tue: () => t('dayTue'),
  wed: () => t('dayWed'),
  thu: () => t('dayThu'),
  fri: () => t('dayFri'),
  sat: () => t('daySat'),
  sun: () => t('daySun'),
};

interface DayHoursState {
  isOpen: boolean;
  open: string;
  close: string;
}

export function BranchesScreen(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveProfileWrite = keyedWrite<unknown, AdminBranchProfileData>(
    api,
    'PUT',
    '/api/branches/profile',
  );

  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [selectedBranch, setSelectedBranch] = createSignal<string>('');
  const [profile, setProfile] = createSignal<AdminBranchProfileData | null>(null);
  const [loading, setLoading] = createSignal<boolean>(false);
  const [saving, setSaving] = createSignal<boolean>(false);
  const [error, setError] = createSignal<string | null>(null);
  const [success, setSuccess] = createSignal<string | null>(null);

  // Form fields
  const [code, setCode] = createSignal<string>('');
  const [address, setAddress] = createSignal<string>('');
  const [timezone, setTimezone] = createSignal<string>('');
  const [bookingEnabled, setBookingEnabled] = createSignal<boolean>(false);

  // Hours state for each day
  const [hours, setHours] = createSignal<Record<DayKey, DayHoursState>>({
    mon: { isOpen: true, open: '09:00', close: '17:00' },
    tue: { isOpen: true, open: '09:00', close: '17:00' },
    wed: { isOpen: true, open: '09:00', close: '17:00' },
    thu: { isOpen: true, open: '09:00', close: '17:00' },
    fri: { isOpen: true, open: '09:00', close: '17:00' },
    sat: { isOpen: false, open: '09:00', close: '17:00' },
    sun: { isOpen: false, open: '09:00', close: '17:00' },
  });

  let lastLoadedBranch = '';
  // #2247: only the latest profile read for the selected branch may fill the form.
  let profileSeq = 0;

  const loadBranches = async () => {
    try {
      const res = await api.view<AdminBranchesData>('admin_branches', {
        branch: props.branch(),
      });
      if (res.data?.branches) {
        setBranches(res.data.branches);
        if (!selectedBranch() && res.data.branches.length > 0) {
          const match = res.data.branches.find((b) => b.id === props.branch());
          setSelectedBranch(match ? match.id : res.data.branches[0]!.id);
        }
      }
    } catch (err: unknown) {
      setError(wordedError(err));
    }
  };

  const loadProfile = async (branchId: string) => {
    if (!branchId) return;
    const seq = ++profileSeq;
    const isStale = () => seq !== profileSeq || branchId !== selectedBranch();
    setLoading(true);
    setError(null);
    setSuccess(null);
    if (branchId !== lastLoadedBranch) {
      setProfile(null);
    }
    try {
      const res = await api.view<AdminBranchProfileData>('admin_branches_profile', {
        branch: branchId,
      });
      if (isStale()) return;
      if (res.data) {
        lastLoadedBranch = branchId;
        setProfile(res.data);
        setCode(res.data.code ?? '');
        setAddress(res.data.address ?? '');
        setTimezone(res.data.timezone ?? '');
        setBookingEnabled(res.data.bookingEnabled);

        const newHours: Record<DayKey, DayHoursState> = { ...hours() };
        for (const day of DAYS) {
          const dayData = res.data.openingHours?.[day];
          if (dayData && dayData.length > 0 && dayData[0]) {
            newHours[day] = {
              isOpen: true,
              open: dayData[0].open,
              close: dayData[0].close,
            };
          } else {
            newHours[day] = {
              isOpen: false,
              open: '09:00',
              close: '17:00',
            };
          }
        }
        setHours(newHours);
      }
    } catch (err: unknown) {
      if (isStale()) return;
      if (branchId !== lastLoadedBranch) {
        setProfile(null);
      }
      setError(wordedError(err));
    } finally {
      if (seq === profileSeq) setLoading(false);
    }
  };

  createEffect(() => {
    if (!props.shown()) return;
    loadBranches();
  });

  createEffect(() => {
    const cur = selectedBranch();
    if (cur) {
      loadProfile(cur);
    }
  });

  const handleDayToggle = (day: DayKey, checked: boolean) => {
    setHours((prev) => ({
      ...prev,
      [day]: {
        ...prev[day],
        isOpen: checked,
      },
    }));
  };

  const handleTimeChange = (day: DayKey, field: 'open' | 'close', value: string) => {
    setHours((prev) => ({
      ...prev,
      [day]: {
        ...prev[day],
        [field]: value,
      },
    }));
  };

  const handleSave = async (e: Event) => {
    e.preventDefault();
    const branchId = selectedBranch();
    if (!branchId) return;

    const tz = timezone().trim();
    if (!tz) {
      setError(t('timezoneRequired'));
      return;
    }

    setError(null);
    setSuccess(null);
    setSaving(true);

    const openingHours: Record<string, Array<{ open: string; close: string }>> = {};
    const h = hours();
    for (const day of DAYS) {
      openingHours[day] = h[day].isOpen
        ? [{ open: h[day].open, close: h[day].close }]
        : [];
    }

    const payload = {
      branch: branchId,
      code: code().trim(),
      address: address().trim() || null,
      timezone: tz,
      openingHours,
      bookingEnabled: bookingEnabled(),
    };

    try {
      const res = await saveProfileWrite.send(payload);
      setProfile(res);
      setSuccess(t('saved'));
    } catch (err: unknown) {
      if (err instanceof ApiError && err.code === 'conflict') {
        setError(t('codeConflict'));
      } else {
        setError(wordedError(err));
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <div class="branches-screen">
      <div class="branches-toolbar">
        <div class="branches-select-group">
          <label class="branches-label" for="branch-select">{t('branchSelect')}:</label>
          <select
            id="branch-select"
            class="branches-select"
            value={selectedBranch()}
            onChange={(e) => setSelectedBranch(e.currentTarget.value)}
          >
            <For each={branches()}>
              {(b) => <option value={b.id}>{b.name} ({b.id})</option>}
            </For>
          </select>
        </div>
        <button
          type="button"
          class="branches-btn"
          onClick={() => {
            loadBranches();
            if (selectedBranch()) loadProfile(selectedBranch());
          }}
        >
          {t('refresh')}
        </button>
      </div>

      <div class="branches-content">
        <Show when={error()}>
          <div role="alert" class="branches-form-error">{error()}</div>
        </Show>
        <Show when={success()}>
          <div role="status" class="branches-form-success">{success()}</div>
        </Show>

        <Show
          when={profile()}
          fallback={<div class="branches-empty">{loading() ? t('saving') : t('noBranches')}</div>}
        >
          {(prof) => (
            <div class="branches-card">
              <h2 class="branches-heading">{t('profileHeading')}: {prof().name}</h2>
              <form class="branches-form" onSubmit={handleSave}>
                <div class="branches-field">
                  <label class="branches-label" for="branch-code">{t('code')}</label>
                  <input
                    id="branch-code"
                    name="code"
                    class="branches-input"
                    value={code()}
                    onInput={(e) => setCode(e.currentTarget.value)}
                    required
                  />
                </div>

                <div class="branches-field">
                  <label class="branches-label" for="branch-address">{t('address')}</label>
                  <input
                    id="branch-address"
                    name="address"
                    class="branches-input"
                    value={address()}
                    onInput={(e) => setAddress(e.currentTarget.value)}
                  />
                </div>

                <div class="branches-field">
                  <label class="branches-label" for="branch-timezone">{t('timezone')}</label>
                  <input
                    id="branch-timezone"
                    name="timezone"
                    class="branches-input"
                    value={timezone()}
                    onInput={(e) => setTimezone(e.currentTarget.value)}
                    required
                  />
                </div>

                <div class="branches-field">
                  <label class="branches-field-checkbox" for="branch-booking">
                    <input
                      id="branch-booking"
                      name="bookingEnabled"
                      type="checkbox"
                      checked={bookingEnabled()}
                      onChange={(e) => setBookingEnabled(e.currentTarget.checked)}
                    />
                    <span class="branches-label">{t('bookingEnabled')}</span>
                  </label>
                </div>

                <div class="branches-hours-section">
                  <h3 class="branches-hours-heading">{t('openingHours')}</h3>
                  <div class="branches-hours-grid">
                    <For each={DAYS}>
                      {(day) => {
                        const h = () => hours()[day];
                        return (
                          <div class="branches-hours-row">
                            <span class="branches-hours-day">{DAY_LABELS[day]()}</span>
                            <label class="branches-hours-toggle">
                              <input
                                type="checkbox"
                                checked={h().isOpen}
                                onChange={(e) => handleDayToggle(day, e.currentTarget.checked)}
                              />
                              <span class="branches-label">{h().isOpen ? t('open') : t('closed')}</span>
                            </label>
                            <Show
                              when={h().isOpen}
                              fallback={<span class="branches-hours-closed">{t('closed')}</span>}
                            >
                              <div class="branches-hours-inputs">
                                <input
                                  type="time"
                                  class="branches-time-input"
                                  value={h().open}
                                  onChange={(e) => handleTimeChange(day, 'open', e.currentTarget.value)}
                                  aria-label={`${DAY_LABELS[day]()} ${t('openTime')}`}
                                />
                                <span>–</span>
                                <input
                                  type="time"
                                  class="branches-time-input"
                                  value={h().close}
                                  onChange={(e) => handleTimeChange(day, 'close', e.currentTarget.value)}
                                  aria-label={`${DAY_LABELS[day]()} ${t('closeTime')}`}
                                />
                              </div>
                            </Show>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </div>

                <div class="branches-actions">
                  <button
                    type="submit"
                    class="branches-btn branches-btn-primary"
                    disabled={saving()}
                  >
                    {saving() ? t('saving') : t('save')}
                  </button>
                </div>
              </form>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
