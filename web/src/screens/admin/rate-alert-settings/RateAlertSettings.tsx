import {
  createEffect,
  createSignal,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  RateAlertSettingsData,
  SaveRateAlertSettingsInput,
} from '../../../shared/api/generated/rate_alert_settings';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './rate-alert-settings.css';

const t = screenT('admin.rate-alert-settings', table);

const SUGGESTED = {
  cooldownHours: '24',
  maxActive: '10',
  lifetimeDays: '30',
  observePolicy: 'ALL_FRESH',
};

export function RateAlertSettings(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<SaveRateAlertSettingsInput, RateAlertSettingsData>(
    api,
    'PUT',
    '/api/rate-alert-settings',
  );

  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [isEmpty, setIsEmpty] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [savedSuccess, setSavedSuccess] = createSignal<string | null>(null);

  const [cooldownHours, setCooldownHours] = createSignal('');
  const [maxActive, setMaxActive] = createSignal('');
  const [lifetimeDays, setLifetimeDays] = createSignal('');
  const [observePolicy, setObservePolicy] = createSignal('');

  const [cooldownError, setCooldownError] = createSignal<string | null>(null);
  const [maxActiveError, setMaxActiveError] = createSignal<string | null>(null);
  const [lifetimeError, setLifetimeError] = createSignal<string | null>(null);
  const [policyError, setPolicyError] = createSignal<string | null>(null);

  let activeBranch = '';

  const clearForm = () => {
    setIsEmpty(false);
    setCooldownHours('');
    setMaxActive('');
    setLifetimeDays('');
    setObservePolicy('');
    setCooldownError(null);
    setMaxActiveError(null);
    setLifetimeError(null);
    setPolicyError(null);
    setSaveError(null);
    setSavedSuccess(null);
  };

  const loadSettings = async (branch: string = props.branch()) => {
    activeBranch = branch;
    setLoading(true);
    setLoadError(null);
    try {
      const res = await api.view<RateAlertSettingsData | null>('rate_alert_settings', {
        branch,
      });
      if (activeBranch !== branch) return;
      if (res.data) {
        setIsEmpty(false);
        setCooldownHours(String(res.data.defaultCooldownSeconds / 3600));
        setMaxActive(String(res.data.maxActivePerCustomer));
        setLifetimeDays(
          res.data.defaultLifetimeDays !== undefined && res.data.defaultLifetimeDays !== null
            ? String(res.data.defaultLifetimeDays)
            : '',
        );
        setObservePolicy(res.data.observePolicy);
      } else {
        setIsEmpty(true);
        setCooldownHours('');
        setMaxActive('');
        setLifetimeDays('');
        setObservePolicy('');
      }
    } catch (err: unknown) {
      if (activeBranch !== branch) return;
      clearForm();
      setLoadError(wordedError(err));
    } finally {
      if (activeBranch === branch) {
        setLoading(false);
      }
    }
  };

  createEffect(() => {
    const branch = props.branch();
    const shown = props.shown();
    if (!shown) return;
    if (branch !== activeBranch) {
      clearForm();
      saveWrite.reset();
    }
    void loadSettings(branch);
  });

  const handleUseSuggested = () => {
    setCooldownHours(SUGGESTED.cooldownHours);
    setMaxActive(SUGGESTED.maxActive);
    setLifetimeDays(SUGGESTED.lifetimeDays);
    setObservePolicy(SUGGESTED.observePolicy);
    setCooldownError(null);
    setMaxActiveError(null);
    setLifetimeError(null);
    setPolicyError(null);
    setSaveError(null);
  };

  const handleSave = async (e: Event) => {
    e.preventDefault();
    if (saving()) return;

    setSavedSuccess(null);
    setSaveError(null);
    setCooldownError(null);
    setMaxActiveError(null);
    setLifetimeError(null);
    setPolicyError(null);

    let hasError = false;

    // Validate Cooldown Hours (> 0 number, <= 720)
    const cdTrim = cooldownHours().trim();
    const cdNum = Number(cdTrim);
    const cdSeconds = Math.round(cdNum * 3600);
    if (!cdTrim || isNaN(cdNum) || cdSeconds < 1) {
      setCooldownError(t('errorCooldownInvalid'));
      hasError = true;
    } else if (cdNum > 720) {
      setCooldownError(t('errorCooldownMax'));
      hasError = true;
    }

    // Validate Max Active (> 0 integer, <= 100)
    const maxTrim = maxActive().trim();
    const maxNum = Number(maxTrim);
    if (!maxTrim || isNaN(maxNum) || !Number.isInteger(maxNum) || maxNum <= 0) {
      setMaxActiveError(t('errorMaxActiveInvalid'));
      hasError = true;
    } else if (maxNum > 100) {
      setMaxActiveError(t('errorMaxActiveMax'));
      hasError = true;
    }

    // Validate Lifetime Days (optional, but if provided must be > 0 integer and <= 365)
    const ltTrim = lifetimeDays().trim();
    let ltVal: number | undefined = undefined;
    if (ltTrim !== '') {
      const ltNum = Number(ltTrim);
      if (isNaN(ltNum) || !Number.isInteger(ltNum) || ltNum <= 0) {
        setLifetimeError(t('errorLifetimeInvalid'));
        hasError = true;
      } else if (ltNum > 365) {
        setLifetimeError(t('errorLifetimeMax'));
        hasError = true;
      } else {
        ltVal = ltNum;
      }
    }

    // Validate Observe Policy (must be selected)
    const pol = observePolicy().trim();
    if (!pol) {
      setPolicyError(t('errorPolicyRequired'));
      hasError = true;
    }

    if (hasError) return;

    setSaving(true);

    const payload: SaveRateAlertSettingsInput = {
      defaultCooldownSeconds: cdSeconds,
      maxActivePerCustomer: maxNum,
      observePolicy: pol,
    };
    if (ltVal !== undefined) {
      payload.defaultLifetimeDays = ltVal;
    }

    try {
      const res = await saveWrite.send(payload);
      setIsEmpty(false);
      setCooldownHours(String(res.defaultCooldownSeconds / 3600));
      setMaxActive(String(res.maxActivePerCustomer));
      setLifetimeDays(
        res.defaultLifetimeDays !== undefined && res.defaultLifetimeDays !== null
          ? String(res.defaultLifetimeDays)
          : '',
      );
      setObservePolicy(res.observePolicy);
      setSavedSuccess(t('saved'));
    } catch (err: unknown) {
      const fe = fieldError(err, {
        defaultCooldownSeconds: t('cooldownLabel'),
        cooldownHours: t('cooldownLabel'),
        maxActivePerCustomer: t('maxActiveLabel'),
        maxActive: t('maxActiveLabel'),
        defaultLifetimeDays: t('lifetimeLabel'),
        lifetimeDays: t('lifetimeLabel'),
        observePolicy: t('observePolicyLabel'),
      });
      if (fe.field === 'defaultCooldownSeconds' || fe.field === 'cooldownHours') {
        setCooldownError(fe.message);
      } else if (fe.field === 'maxActivePerCustomer' || fe.field === 'maxActive') {
        setMaxActiveError(fe.message);
      } else if (fe.field === 'defaultLifetimeDays' || fe.field === 'lifetimeDays') {
        setLifetimeError(fe.message);
      } else if (fe.field === 'observePolicy') {
        setPolicyError(fe.message);
      } else {
        setSaveError(fe.message);
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <div class="rate-alert-settings">
      <Show when={loading()}>
        <div class="rate-alert-loading" data-state="loading">
          {t('loading')}
        </div>
      </Show>

      <Show when={!loading() && loadError()}>
        <div class="rate-alert-error" data-state="error" role="alert">
          <span>{loadError()}</span>
          <button
            type="button"
            class="rate-alert-retry-btn"
            data-action="retry"
            onClick={() => void loadSettings()}
          >
            {t('retry')}
          </button>
        </div>
      </Show>

      <Show when={!loading() && !loadError()}>
        <div data-state={isEmpty() ? 'empty' : 'loaded'}>
          <Show when={isEmpty()}>
            <div class="rate-alert-banner" data-state="empty">
              <div class="rate-alert-banner-title">{t('emptyTitle')}</div>
              <div class="rate-alert-banner-text">{t('emptyNote')}</div>
            </div>
          </Show>

          <Show when={savedSuccess()}>
            <div class="rate-alert-success" role="status">
              {savedSuccess()}
            </div>
          </Show>

          <div class="rate-alert-card">
            <div class="rate-alert-card-title">{t('title')}</div>

            <form class="rate-alert-form" noValidate onSubmit={handleSave}>
              <div class="rate-alert-field">
                <label class="rate-alert-label" for="default-cooldown-hours">
                  <span>{t('cooldownLabel')}</span>
                  <span class="rate-alert-suggestion">
                    {t('suggested')}: {SUGGESTED.cooldownHours}
                  </span>
                </label>
                <input
                  id="default-cooldown-hours"
                  name="defaultCooldownHours"
                  class="rate-alert-input"
                  type="number"
                  min="0.001"
                  max="720"
                  step="any"
                  placeholder={SUGGESTED.cooldownHours}
                  value={cooldownHours()}
                  onInput={(e) => {
                    setCooldownHours(e.currentTarget.value);
                    setCooldownError(null);
                    setSaveError(null);
                  }}
                  disabled={saving()}
                />
                <span class="rate-alert-hint">{t('cooldownHint')}</span>
                <Show when={cooldownError()}>
                  <span class="rate-alert-field-error" data-error="cooldown">
                    {cooldownError()}
                  </span>
                </Show>
              </div>

              <div class="rate-alert-field">
                <label class="rate-alert-label" for="max-active-per-customer">
                  <span>{t('maxActiveLabel')}</span>
                  <span class="rate-alert-suggestion">
                    {t('suggested')}: {SUGGESTED.maxActive}
                  </span>
                </label>
                <input
                  id="max-active-per-customer"
                  name="maxActivePerCustomer"
                  class="rate-alert-input"
                  type="number"
                  min="1"
                  max="100"
                  step="1"
                  placeholder={SUGGESTED.maxActive}
                  value={maxActive()}
                  onInput={(e) => {
                    setMaxActive(e.currentTarget.value);
                    setMaxActiveError(null);
                    setSaveError(null);
                  }}
                  disabled={saving()}
                />
                <span class="rate-alert-hint">{t('maxActiveHint')}</span>
                <Show when={maxActiveError()}>
                  <span class="rate-alert-field-error" data-error="maxActive">
                    {maxActiveError()}
                  </span>
                </Show>
              </div>

              <div class="rate-alert-field">
                <label class="rate-alert-label" for="default-lifetime-days">
                  <span>{t('lifetimeLabel')}</span>
                  <span class="rate-alert-suggestion">
                    {t('suggested')}: {SUGGESTED.lifetimeDays}
                  </span>
                </label>
                <input
                  id="default-lifetime-days"
                  name="defaultLifetimeDays"
                  class="rate-alert-input"
                  type="number"
                  min="1"
                  max="365"
                  step="1"
                  placeholder={SUGGESTED.lifetimeDays}
                  value={lifetimeDays()}
                  onInput={(e) => {
                    setLifetimeDays(e.currentTarget.value);
                    setLifetimeError(null);
                    setSaveError(null);
                  }}
                  disabled={saving()}
                />
                <span class="rate-alert-hint">{t('lifetimeHint')}</span>
                <Show when={lifetimeError()}>
                  <span class="rate-alert-field-error" data-error="lifetime">
                    {lifetimeError()}
                  </span>
                </Show>
              </div>

              <div class="rate-alert-field">
                <div class="rate-alert-label">
                  <span>{t('observePolicyLabel')}</span>
                  <span class="rate-alert-suggestion">
                    {t('suggested')}: {SUGGESTED.observePolicy}
                  </span>
                </div>
                <div class="rate-alert-policies">
                  <label class="rate-alert-policy-option">
                    <input
                      type="radio"
                      name="observePolicy"
                      value="ALL_FRESH"
                      checked={observePolicy() === 'ALL_FRESH'}
                      onChange={() => {
                        setObservePolicy('ALL_FRESH');
                        setPolicyError(null);
                        setSaveError(null);
                      }}
                      disabled={saving()}
                    />
                    <div>
                      <div class="rate-alert-policy-label">{t('policyAllFreshTitle')}</div>
                      <div class="rate-alert-policy-desc">{t('policyAllFreshDesc')}</div>
                    </div>
                  </label>

                  <label class="rate-alert-policy-option">
                    <input
                      type="radio"
                      name="observePolicy"
                      value="ALL"
                      checked={observePolicy() === 'ALL'}
                      onChange={() => {
                        setObservePolicy('ALL');
                        setPolicyError(null);
                        setSaveError(null);
                      }}
                      disabled={saving()}
                    />
                    <div>
                      <div class="rate-alert-policy-label">{t('policyAllTitle')}</div>
                      <div class="rate-alert-policy-desc">{t('policyAllDesc')}</div>
                    </div>
                  </label>

                  <label class="rate-alert-policy-option">
                    <input
                      type="radio"
                      name="observePolicy"
                      value="AUTO_FRESH"
                      checked={observePolicy() === 'AUTO_FRESH'}
                      onChange={() => {
                        setObservePolicy('AUTO_FRESH');
                        setPolicyError(null);
                        setSaveError(null);
                      }}
                      disabled={saving()}
                    />
                    <div>
                      <div class="rate-alert-policy-label">{t('policyAutoFreshTitle')}</div>
                      <div class="rate-alert-policy-desc">{t('policyAutoFreshDesc')}</div>
                    </div>
                  </label>
                </div>
                <Show when={policyError()}>
                  <span class="rate-alert-field-error" data-error="observePolicy">
                    {policyError()}
                  </span>
                </Show>
              </div>

              <Show when={saveError()}>
                <div class="rate-alert-save-error" data-error="save" role="alert">
                  {saveError()}
                </div>
              </Show>

              <div class="rate-alert-actions">
                <Show when={isEmpty()}>
                  <button
                    type="button"
                    class="rate-alert-suggest-btn"
                    onClick={handleUseSuggested}
                    disabled={saving()}
                  >
                    {t('useSuggested')}
                  </button>
                </Show>
                <button
                  type="submit"
                  class="rate-alert-submit-btn"
                  disabled={saving()}
                >
                  {saving() ? t('saving') : t('save')}
                </button>
              </div>
            </form>
          </div>
        </div>
      </Show>
    </div>
  );
}
