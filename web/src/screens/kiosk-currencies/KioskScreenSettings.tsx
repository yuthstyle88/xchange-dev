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

const EMPTY_VALUES: Record<KioskScreenField, string> = {
  quickAmounts: '',
  tiles: '',
  popularDays: '',
  idleSeconds: '',
  reviewSeconds: '',
  refreshSeconds: '',
  defaultPaymentMethod: '',
};

function parseInteger(raw: string): number {
  const trimmed = raw.trim();
  if (!/^-?[0-9]+$/.test(trimmed)) return -1;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) ? n : -1;
}

function parseQuickAmountsInput(raw: string): number[] {
  const parts = raw
    .split(/[,;\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  return parts.map((p) => {
    if (!/^-?[0-9]+$/.test(p)) return 0;
    const n = Number(p);
    return Number.isSafeInteger(n) ? n : 0;
  });
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
    ...EMPTY_VALUES,
  });
  const [serverDefaults, setServerDefaults] = createSignal<
    Record<KioskScreenField, string>
  >({
    ...EMPTY_VALUES,
  });
  const [paymentMethods, setPaymentMethods] = createSignal<string[]>([]);
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
    setPaymentMethods(data.paymentMethods);
    setServerDefaults({
      quickAmounts: data.defaultValues.quickAmounts.join(', '),
      tiles: String(data.defaultValues.tiles),
      popularDays: String(data.defaultValues.popularDays),
      idleSeconds: String(data.defaultValues.idleSeconds),
      reviewSeconds: String(data.defaultValues.reviewSeconds),
      refreshSeconds: String(data.defaultValues.refreshSeconds),
      defaultPaymentMethod: data.defaultValues.defaultPaymentMethod,
    });
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
        setLoaded(true);
      } else {
        setValues({ ...EMPTY_VALUES });
        setServerDefaults({ ...EMPTY_VALUES });
        setPaymentMethods([]);
        setDefaults(new Set(ALL_FIELDS));
        setVersion(null);
        setLoaded(false);
      }
      setFieldErrors({});
      setError(undefined);
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
    setValues({ ...EMPTY_VALUES });
    setServerDefaults({ ...EMPTY_VALUES });
    setPaymentMethods([]);
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
    setValues((prev) => ({ ...prev, [field]: serverDefaults()[field] }));
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
    setFieldErrors({});
    const curValues = values();
    const curDefaults = defaults();

    const input: KioskScreenSaveInput = {
      branch: branchId,
      quickAmounts: curDefaults.has('quickAmounts')
        ? null
        : parseQuickAmountsInput(curValues.quickAmounts),
      tiles: curDefaults.has('tiles') ? null : parseInteger(curValues.tiles),
      idleSeconds: curDefaults.has('idleSeconds')
        ? null
        : parseInteger(curValues.idleSeconds),
      reviewSeconds: curDefaults.has('reviewSeconds')
        ? null
        : parseInteger(curValues.reviewSeconds),
      refreshSeconds: curDefaults.has('refreshSeconds')
        ? null
        : parseInteger(curValues.refreshSeconds),
      popularDays: curDefaults.has('popularDays')
        ? null
        : parseInteger(curValues.popularDays),
      defaultPaymentMethod: curDefaults.has('defaultPaymentMethod')
        ? null
        : curValues.defaultPaymentMethod.trim(),
      expectedVersion: version(),
    };

    setSaving(true);
    const seq = ++readSeq;

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
            <For each={paymentMethods()}>
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
