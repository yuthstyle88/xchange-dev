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

  const [staleDataMinutes, setStaleDataMinutes] = createSignal<string>('');
  const [minutesError, setMinutesError] = createSignal<string | null>(null);
  const [floors, setFloors] = createSignal<StockFloorDto[]>([]);
  const [floorErrors, setFloorErrors] = createSignal<Record<string, string>>({});
  const [newCurrency, setNewCurrency] = createSignal<string>('');
  const [addCurrencyError, setAddCurrencyError] = createSignal<string | null>(null);
  const [version, setVersion] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal<boolean>(false);
  const [firstLoaded, setFirstLoaded] = createSignal<boolean>(false);

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
        setStaleDataMinutes(
          res.data.staleDataMinutes === null ? '' : String(res.data.staleDataMinutes),
        );
        setFloors(res.data.floors ? res.data.floors.map((f) => ({ ...f })) : []);
        setVersion(res.data.version ?? null);
      } else {
        setStaleDataMinutes('');
        setFloors([]);
        setVersion(null);
      }
      setMinutesError(null);
      setFloorErrors({});
      setAddCurrencyError(null);
      setFirstLoaded(true);
    } catch (err: unknown) {
      if (activeBranch !== branch || seq !== readSeq) return;
      setStaleDataMinutes('');
      setFloors([]);
      setVersion(null);
      setMinutesError(null);
      setFloorErrors({});
      setAddCurrencyError(null);
      setError(wordedError(err));
      setFirstLoaded(true);
    }
  };

  createEffect(() => {
    const shown = props.shown();
    const branch = props.branch();
    const tick = refreshTick();
    if (!shown) return;
    if (branch !== activeBranch) {
      setStaleDataMinutes('');
      setFloors([]);
      setVersion(null);
      setMinutesError(null);
      setFloorErrors({});
      setAddCurrencyError(null);
      setSaving(false);
      setFirstLoaded(false);
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
    setFloorErrors((prev) => {
      if (!(currency in prev)) return prev;
      const next = { ...prev };
      delete next[currency];
      return next;
    });
  };

  const handleAddFloor = () => {
    setAddCurrencyError(null);
    const code = newCurrency().trim().toUpperCase();
    if (!/^[A-Z]{3}$/.test(code)) {
      setAddCurrencyError(t('invalidCurrencyCode'));
      return;
    }
    if (floors().some((f) => f.currency === code)) {
      return;
    }
    setFloors((prev) => [...prev, { currency: code, minQuantity: '' }]);
    setNewCurrency('');
  };

  const handleRemoveFloor = (currency: string) => {
    setFloors((prev) => prev.filter((f) => f.currency !== currency));
    setFloorErrors((prev) => {
      if (!(currency in prev)) return prev;
      const next = { ...prev };
      delete next[currency];
      return next;
    });
  };

  const handleSave = async () => {
    setError(null);
    setMinutesError(null);
    setFloorErrors({});

    let hasError = false;
    const rawMinutes = staleDataMinutes().trim();
    let parsedMinutes: number | null = null;
    if (rawMinutes !== '') {
      if (
        !/^[1-9][0-9]*$/.test(rawMinutes) ||
        BigInt(rawMinutes) > 2147483647n
      ) {
        setMinutesError(t('invalidMinutes'));
        hasError = true;
      } else {
        parsedMinutes = Number(rawMinutes);
      }
    }

    const nextFloorErrors: Record<string, string> = {};
    for (const f of floors()) {
      if (!/^[0-9]+(\.[0-9]{1,4})?$/.test(f.minQuantity)) {
        nextFloorErrors[f.currency] = t('invalidFloorQuantity');
        hasError = true;
      }
    }
    if (hasError) {
      setFloorErrors(nextFloorErrors);
      return;
    }

    setSaving(true);
    const seq = ++readSeq;

    const branch = props.branch();
    const isStale = () => props.branch() !== branch || seq !== readSeq;
    const input: BranchAlertSettingsSaveInput = {
      branch,
      staleDataMinutes: parsedMinutes,
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
      setStaleDataMinutes(
        res.staleDataMinutes === null ? '' : String(res.staleDataMinutes),
      );
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
              type="text"
              inputMode="numeric"
              disabled={!firstLoaded()}
              value={staleDataMinutes()}
              onInput={(e) => {
                setStaleDataMinutes(e.currentTarget.value);
                setMinutesError(null);
              }}
            />
          </label>
          <Show when={minutesError()}>
            <div class="alert-settings-error" role="alert">
              {minutesError()}
            </div>
          </Show>
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
                    disabled={!firstLoaded()}
                    value={floor().minQuantity}
                    onInput={(e) => updateFloor(floor().currency, e.currentTarget.value)}
                  />
                  <button
                    type="button"
                    class="remove-floor-button"
                    disabled={!firstLoaded()}
                    onClick={() => handleRemoveFloor(floor().currency)}
                  >
                    {t('removeFloor')}
                  </button>
                  <Show when={floorErrors()[floor().currency]}>
                    <div class="alert-settings-error" role="alert">
                      {floorErrors()[floor().currency]}
                    </div>
                  </Show>
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
                disabled={!firstLoaded()}
                value={newCurrency()}
                onInput={(e) => {
                  setNewCurrency(e.currentTarget.value);
                  setAddCurrencyError(null);
                }}
              />
            </label>
            <button
              type="button"
              class="add-floor-button"
              disabled={!firstLoaded()}
              onClick={handleAddFloor}
            >
              {t('addFloor')}
            </button>
            <Show when={addCurrencyError()}>
              <div class="alert-settings-error" role="alert">
                {addCurrencyError()}
              </div>
            </Show>
          </div>
        </div>

        <div class="alert-settings-actions">
          <button
            type="button"
            disabled={!firstLoaded() || saving()}
            onClick={handleSave}
          >
            {t('save')}
          </button>
        </div>
      </div>
    </div>
  );
}
