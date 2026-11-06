import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../shared/api/client';
import type {
  KioskScreenData,
  KioskScreenSaveInput,
} from '../../shared/api/generated/kiosk_screen';
import { keyedWrite } from '../../shared/api/keyed-write';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import * as screenI18n from './i18n';
import { KIOSK_SCREEN_SAVE_PATH, readKioskScreen } from './kiosk-api';

export type KioskScreenField =
  | 'quickAmounts'
  | 'tiles'
  | 'popularDays'
  | 'idleSeconds'
  | 'reviewSeconds'
  | 'refreshSeconds'
  | 'defaultPaymentMethod';

const ALL_FIELDS: readonly KioskScreenField[] = [
  'quickAmounts',
  'tiles',
  'popularDays',
  'idleSeconds',
  'reviewSeconds',
  'refreshSeconds',
  'defaultPaymentMethod',
];

const PAYMENT_METHODS = [
  'Cash',
  'Credit Card',
  'Alipay',
  'PromptPay',
  'Bank Transfer',
] as const;

const DEFAULT_VALUES: Record<KioskScreenField, string> = {
  quickAmounts: '100, 500, 1000, 5000',
  tiles: '6',
  popularDays: '30',
  idleSeconds: '120',
  reviewSeconds: '3',
  refreshSeconds: '10',
  defaultPaymentMethod: 'Cash',
};

function parseWholeInRange(
  raw: string,
  min: number,
  max: number,
): number | null {
  const trimmed = raw.trim();
  if (!/^[0-9]+$/.test(trimmed)) return null;
  const n = Number(trimmed);
  if (!Number.isSafeInteger(n) || n < min || n > max) return null;
  return n;
}

function parseQuickAmounts(raw: string): number[] | null {
  const parts = raw
    .split(/[,;\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  if (parts.length < 1 || parts.length > 8) return null;
  const nums: number[] = [];
  for (const part of parts) {
    const n = parseWholeInRange(part, 1, 10_000_000);
    if (n === null) return null;
    nums.push(n);
  }
  return nums;
}

export function KioskScreenSettings(props: {
  api: Pick<ApiClient, 'view'> & Partial<Pick<ApiClient, 'command'>>;
  branch?: string;
  autoLoad?: boolean;
}): JSX.Element {
  const t = screenT('kiosk-currencies', screenI18n);

  const clientWithCommand: ApiClient =
    typeof props.api.command === 'function'
      ? (props.api as ApiClient)
      : new ApiClient();
  const saveWrite = keyedWrite<KioskScreenSaveInput, KioskScreenData>(
    clientWithCommand,
    'POST',
    KIOSK_SCREEN_SAVE_PATH,
  );

  const [values, setValues] = createSignal<Record<KioskScreenField, string>>({
    ...DEFAULT_VALUES,
  });
  const [defaults, setDefaults] = createSignal<Set<KioskScreenField>>(
    new Set(ALL_FIELDS),
  );
  const [version, setVersion] = createSignal<number | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>(undefined);
  const [savedNotice, setSavedNotice] = createSignal<string | undefined>(
    undefined,
  );
  const [fieldErrors, setFieldErrors] = createSignal<
    Partial<Record<KioskScreenField, string>>
  >({});

  let activeBranch = '';
  let readSeq = 0;

  const applyData = (data: KioskScreenData) => {
    setValues({
      quickAmounts: data.quickAmounts.join(', '),
      tiles: String(data.tiles),
      popularDays: String(data.popularDays),
      idleSeconds: String(data.idleSeconds),
      reviewSeconds: String(data.reviewSeconds),
      refreshSeconds: String(data.refreshSeconds),
      defaultPaymentMethod: data.defaultPaymentMethod,
    });
    setDefaults(
      new Set(
        data.defaults.filter((f): f is KioskScreenField =>
          (ALL_FIELDS as readonly string[]).includes(f),
        ),
      ),
    );
    setVersion(data.version ?? null);
  };

  const loadScreen = async (branchId: string) => {
    if (!branchId) return;
    activeBranch = branchId;
    const seq = ++readSeq;
    try {
      const data = await readKioskScreen(props.api, branchId);
      if (activeBranch !== branchId || seq !== readSeq) return;
      if (data) {
        applyData(data);
      } else {
        setValues({ ...DEFAULT_VALUES });
        setDefaults(new Set(ALL_FIELDS));
        setVersion(null);
      }
      setFieldErrors({});
      setError(undefined);
      setLoaded(true);
    } catch (err: unknown) {
      if (activeBranch !== branchId || seq !== readSeq) return;
      setError(wordedError(err));
      setLoaded(false);
    }
  };

  createEffect(() => {
    const b = props.branch ?? '';
    activeBranch = b;
    readSeq += 1;
    saveWrite.reset();
    setValues({ ...DEFAULT_VALUES });
    setDefaults(new Set(ALL_FIELDS));
    setVersion(null);
    setLoaded(false);
    setSaving(false);
    setError(undefined);
    setSavedNotice(undefined);
    setFieldErrors({});
    if (b && props.autoLoad !== false) {
      void loadScreen(b);
    }
  });

  const updateField = (field: KioskScreenField, nextValue: string) => {
    setValues((prev) => ({ ...prev, [field]: nextValue }));
    setDefaults((prev) => {
      if (!prev.has(field)) return prev;
      const next = new Set(prev);
      next.delete(field);
      return next;
    });
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
    setError(undefined);
    setSavedNotice(undefined);
  };

  const resetField = (field: KioskScreenField) => {
    setValues((prev) => ({ ...prev, [field]: DEFAULT_VALUES[field] }));
    setDefaults((prev) => {
      const next = new Set(prev);
      next.add(field);
      return next;
    });
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
    setError(undefined);
    setSavedNotice(undefined);
  };

  const isDefault = (field: KioskScreenField) => defaults().has(field);

  const handleSave = async (e?: Event) => {
    e?.preventDefault();
    const branchId = props.branch;
    if (!branchId || !loaded() || saving()) return;

    setError(undefined);
    setSavedNotice(undefined);
    const nextErrors: Partial<Record<KioskScreenField, string>> = {};
    const curValues = values();
    const curDefaults = defaults();

    let quickAmounts: number[] | null = null;
    if (!curDefaults.has('quickAmounts')) {
      quickAmounts = parseQuickAmounts(curValues.quickAmounts);
      if (quickAmounts === null) {
        nextErrors.quickAmounts = t('quickAmountsRange');
      }
    }

    let tiles: number | null = null;
    if (!curDefaults.has('tiles')) {
      tiles = parseWholeInRange(curValues.tiles, 1, 12);
      if (tiles === null) {
        nextErrors.tiles = t('tilesRange');
      }
    }

    let popularDays: number | null = null;
    if (!curDefaults.has('popularDays')) {
      popularDays = parseWholeInRange(curValues.popularDays, 1, 365);
      if (popularDays === null) {
        nextErrors.popularDays = t('popularDaysRange');
      }
    }

    let idleSeconds: number | null = null;
    if (!curDefaults.has('idleSeconds')) {
      idleSeconds = parseWholeInRange(curValues.idleSeconds, 30, 1800);
      if (idleSeconds === null) {
        nextErrors.idleSeconds = t('idleSecondsRange');
      }
    }

    let reviewSeconds: number | null = null;
    if (!curDefaults.has('reviewSeconds')) {
      reviewSeconds = parseWholeInRange(curValues.reviewSeconds, 0, 10);
      if (reviewSeconds === null) {
        nextErrors.reviewSeconds = t('reviewSecondsRange');
      }
    }

    let refreshSeconds: number | null = null;
    if (!curDefaults.has('refreshSeconds')) {
      refreshSeconds = parseWholeInRange(curValues.refreshSeconds, 5, 19);
      if (refreshSeconds === null) {
        nextErrors.refreshSeconds = t('refreshSecondsRange');
      }
    }

    let defaultPaymentMethod: string | null = null;
    if (!curDefaults.has('defaultPaymentMethod')) {
      const m = curValues.defaultPaymentMethod.trim();
      if (!m) {
        nextErrors.defaultPaymentMethod = t('defaultPaymentMethodInvalid');
      } else {
        defaultPaymentMethod = m;
      }
    }

    if (Object.keys(nextErrors).length > 0) {
      setFieldErrors(nextErrors);
      return;
    }

    setFieldErrors({});
    setSaving(true);
    const seq = ++readSeq;
    const input: KioskScreenSaveInput = {
      branch: branchId,
      quickAmounts,
      tiles,
      idleSeconds,
      reviewSeconds,
      refreshSeconds,
      popularDays,
      defaultPaymentMethod,
      expectedVersion: version(),
    };

    try {
      const saved = await saveWrite.send(input);
      if (props.branch !== branchId || seq !== readSeq) return;
      applyData(saved);
      setSavedNotice(t('screenSaved'));
    } catch (err: unknown) {
      if (props.branch !== branchId || seq !== readSeq) return;
      if (
        err instanceof ApiError &&
        err.field &&
        (ALL_FIELDS as readonly string[]).includes(err.field)
      ) {
        const f = err.field as KioskScreenField;
        setFieldErrors({ [f]: wordedError(err) });
      } else {
        setError(wordedError(err));
      }
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadScreen(branchId);
      }
    } finally {
      if (props.branch === branchId) {
        setSaving(false);
      }
    }
  };

  const renderNumberRow = (
    field: Exclude<KioskScreenField, 'quickAmounts' | 'defaultPaymentMethod'>,
    id: string,
    labelKey:
      | 'tilesLabel'
      | 'popularDaysLabel'
      | 'idleSecondsLabel'
      | 'reviewSecondsLabel'
      | 'refreshSecondsLabel',
  ) => (
    <div class="kiosk-screen-field" data-field={field}>
      <div class="kiosk-screen-field-header">
        <label for={id}>{t(labelKey)}</label>
        <Show when={isDefault(field)}>
          <span class="kiosk-default-badge" data-default-field={field}>
            {t('defaultBadge')}
          </span>
        </Show>
      </div>
      <div class="kiosk-screen-field-controls">
        <input
          id={id}
          name={field}
          type="text"
          inputmode="numeric"
          value={values()[field]}
          disabled={!loaded() || saving()}
          aria-invalid={fieldErrors()[field] ? 'true' : 'false'}
          onInput={(e) => updateField(field, e.currentTarget.value)}
        />
        <button
          type="button"
          name={`reset_${field}`}
          data-reset-field={field}
          disabled={!loaded() || saving()}
          onClick={() => resetField(field)}
        >
          {t('resetToDefault')}
        </button>
      </div>
      <Show when={fieldErrors()[field]}>
        {(msg) => (
          <span role="alert" class="warning" data-error-field={field}>
            {msg()}
          </span>
        )}
      </Show>
    </div>
  );

  return (
    <section
      aria-label={t('screenHeading')}
      data-section="kiosk-screen"
      class="kiosk-screen-settings"
    >
      <h2>{t('screenHeading')}</h2>

      <Show when={error()}>
        {(msg) => (
          <div role="alert" class="warning">
            {msg()}
          </div>
        )}
      </Show>

      <div class="kiosk-screen-field" data-field="quickAmounts">
        <div class="kiosk-screen-field-header">
          <label for="kiosk-quick-amounts">{t('quickAmountsLabel')}</label>
          <Show when={isDefault('quickAmounts')}>
            <span class="kiosk-default-badge" data-default-field="quickAmounts">
              {t('defaultBadge')}
            </span>
          </Show>
        </div>
        <div class="kiosk-screen-field-controls">
          <input
            id="kiosk-quick-amounts"
            name="quickAmounts"
            type="text"
            value={values().quickAmounts}
            disabled={!loaded() || saving()}
            aria-invalid={fieldErrors().quickAmounts ? 'true' : 'false'}
            onInput={(e) => updateField('quickAmounts', e.currentTarget.value)}
          />
          <button
            type="button"
            name="reset_quickAmounts"
            data-reset-field="quickAmounts"
            disabled={!loaded() || saving()}
            onClick={() => resetField('quickAmounts')}
          >
            {t('resetToDefault')}
          </button>
        </div>
        <Show when={fieldErrors().quickAmounts}>
          {(msg) => (
            <span role="alert" class="warning" data-error-field="quickAmounts">
              {msg()}
            </span>
          )}
        </Show>
      </div>

      {renderNumberRow('tiles', 'kiosk-tiles', 'tilesLabel')}
      {renderNumberRow('popularDays', 'kiosk-popular-days', 'popularDaysLabel')}
      {renderNumberRow('idleSeconds', 'kiosk-idle-seconds', 'idleSecondsLabel')}
      {renderNumberRow(
        'reviewSeconds',
        'kiosk-review-seconds',
        'reviewSecondsLabel',
      )}
      {renderNumberRow(
        'refreshSeconds',
        'kiosk-refresh-seconds',
        'refreshSecondsLabel',
      )}

      <div class="kiosk-screen-field" data-field="defaultPaymentMethod">
        <div class="kiosk-screen-field-header">
          <label for="kiosk-default-payment-method">
            {t('defaultPaymentMethodLabel')}
          </label>
          <Show when={isDefault('defaultPaymentMethod')}>
            <span
              class="kiosk-default-badge"
              data-default-field="defaultPaymentMethod"
            >
              {t('defaultBadge')}
            </span>
          </Show>
        </div>
        <div class="kiosk-screen-field-controls">
          <select
            id="kiosk-default-payment-method"
            name="defaultPaymentMethod"
            value={values().defaultPaymentMethod}
            disabled={!loaded() || saving()}
            aria-invalid={fieldErrors().defaultPaymentMethod ? 'true' : 'false'}
            onChange={(e) =>
              updateField('defaultPaymentMethod', e.currentTarget.value)
            }
          >
            <For each={PAYMENT_METHODS}>
              {(m) => <option value={m}>{m}</option>}
            </For>
          </select>
          <button
            type="button"
            name="reset_defaultPaymentMethod"
            data-reset-field="defaultPaymentMethod"
            disabled={!loaded() || saving()}
            onClick={() => resetField('defaultPaymentMethod')}
          >
            {t('resetToDefault')}
          </button>
        </div>
        <Show when={fieldErrors().defaultPaymentMethod}>
          {(msg) => (
            <span
              role="alert"
              class="warning"
              data-error-field="defaultPaymentMethod"
            >
              {msg()}
            </span>
          )}
        </Show>
      </div>

      <div class="kiosk-screen-actions">
        <button
          type="button"
          name="save_kiosk_screen"
          class="primary"
          disabled={!loaded() || saving()}
          onClick={handleSave}
        >
          {t('screenSave')}
        </button>
        <Show when={savedNotice()}>
          {(msg) => <span role="status">{msg()}</span>}
        </Show>
      </div>
    </section>
  );
}
