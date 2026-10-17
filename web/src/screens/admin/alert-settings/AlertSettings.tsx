import {
  createEffect,
  createSignal,
  Index,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  BranchAlertSettingsData,
  BranchAlertSettingsSaveInput,
  StockFloorDto,
} from '../../../shared/api/generated/branch_alert_settings';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { refreshTick } from '../parts/refresh';
import * as table from './i18n';
import './alert-settings.css';

const t = screenT('admin.alert-settings', table);

export function AlertSettings(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<BranchAlertSettingsSaveInput, BranchAlertSettingsData>(
    api,
    'POST',
    '/api/v3/commands/branch_alert_settings_save',
  );

  const [staleDataMinutes, setStaleDataMinutes] = createSignal<number | null>(null);
  const [floors, setFloors] = createSignal<StockFloorDto[]>([]);
  const [newCurrency, setNewCurrency] = createSignal<string>('');
  const [version, setVersion] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal<boolean>(false);

  let activeBranch = '';
  let lastTick: number | undefined;
  let readSeq = 0;

  const loadSettings = async (branch: string) => {
    if (!branch) return;
    activeBranch = branch;
    const seq = ++readSeq;
    try {
      const res = await api.view<BranchAlertSettingsData>('branch_alert_settings', {
        branch,
      });
      if (activeBranch !== branch || seq !== readSeq) return;
      if (res.data) {
        setStaleDataMinutes(res.data.staleDataMinutes);
        setFloors(res.data.floors ? res.data.floors.map((f) => ({ ...f })) : []);
        setVersion(res.data.version ?? null);
      } else {
        setStaleDataMinutes(null);
        setFloors([]);
        setVersion(null);
      }
    } catch (err: unknown) {
      if (activeBranch !== branch || seq !== readSeq) return;
      setStaleDataMinutes(null);
      setFloors([]);
      setVersion(null);
      setError(wordedError(err));
    }
  };

  createEffect(() => {
    const shown = props.shown();
    const branch = props.branch();
    const tick = refreshTick();
    if (!shown) return;
    if (branch !== activeBranch) {
      setStaleDataMinutes(null);
      setFloors([]);
      setVersion(null);
      setSaving(false);
      saveWrite.reset();
      lastTick = tick;
      setError(null);
      loadSettings(branch);
    } else if (tick !== lastTick) {
      lastTick = tick;
      setError(null);
      loadSettings(branch);
    }
  });

  const updateFloor = (currency: string, minQuantity: string) => {
    setFloors((prev) =>
      prev.map((f) => (f.currency === currency ? { ...f, minQuantity } : f)),
    );
  };

  const handleAddFloor = () => {
    const code = newCurrency().trim().toUpperCase();
    if (!code) return;
    if (floors().some((f) => f.currency === code)) {
      return;
    }
    setFloors((prev) => [...prev, { currency: code, minQuantity: '' }]);
    setNewCurrency('');
  };

  const handleRemoveFloor = (currency: string) => {
    setFloors((prev) => prev.filter((f) => f.currency !== currency));
  };

  const handleSave = async () => {
    setError(null);
    setSaving(true);
    const seq = ++readSeq;

    const branch = props.branch();
    const isStale = () => props.branch() !== branch || seq !== readSeq;
    const input: BranchAlertSettingsSaveInput = {
      branch,
      staleDataMinutes: staleDataMinutes(),
      floors: floors().map((f) => ({
        currency: f.currency,
        minQuantity: f.minQuantity,
      })),
      expectedVersion: version(),
    };

    try {
      const res = await saveWrite.send(input);
      if (isStale()) return;
      setVersion(res.version ?? null);
      setStaleDataMinutes(res.staleDataMinutes);
      setFloors(res.floors ? res.floors.map((f) => ({ ...f })) : []);
    } catch (err: unknown) {
      if (isStale()) return;
      setError(wordedError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadSettings(branch);
      }
    } finally {
      if (props.branch() === branch) {
        setSaving(false);
      }
    }
  };

  return (
    <div class="alert-settings-container">
      <div class="alert-settings-card">
        <Show when={error()}>
          <div class="alert-settings-error" role="alert">
            {error()}
          </div>
        </Show>

        <div class="alert-settings-field">
          <label for="stale-data-minutes">
            {t('staleDataMinutes')}
            <input
              id="stale-data-minutes"
              type="number"
              value={staleDataMinutes() === null ? '' : String(staleDataMinutes())}
              onInput={(e) => {
                const val = e.currentTarget.value.trim();
                if (val === '') {
                  setStaleDataMinutes(null);
                } else {
                  const num = Number(val);
                  setStaleDataMinutes(isNaN(num) ? null : num);
                }
              }}
            />
          </label>
        </div>

        <div class="floors-section">
          <h3>{t('floorsHeading')}</h3>
          <Show
            when={floors().length > 0}
            fallback={<div class="no-floors">{t('noFloors')}</div>}
          >
            <Index each={floors()}>
              {(floor) => (
                <div data-floor={floor().currency} class="floor-row">
                  <span class="floor-currency">{floor().currency}</span>
                  <input
                    type="text"
                    value={floor().minQuantity}
                    onInput={(e) => updateFloor(floor().currency, e.currentTarget.value)}
                  />
                  <button
                    type="button"
                    class="remove-floor-button"
                    onClick={() => handleRemoveFloor(floor().currency)}
                  >
                    {t('removeFloor')}
                  </button>
                </div>
              )}
            </Index>
          </Show>

          <div class="add-floor-section">
            <label for="add-currency" class="add-currency-label">
              {t('addCurrency')}
              <input
                id="add-currency"
                type="text"
                value={newCurrency()}
                onInput={(e) => setNewCurrency(e.currentTarget.value)}
              />
            </label>
            <button
              type="button"
              class="add-floor-button"
              onClick={handleAddFloor}
            >
              {t('addFloor')}
            </button>
          </div>
        </div>

        <div class="alert-settings-actions">
          <button
            type="button"
            disabled={saving()}
            onClick={handleSave}
          >
            {t('save')}
          </button>
        </div>
      </div>
    </div>
  );
}
