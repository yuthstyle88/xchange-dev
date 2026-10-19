// C07 customer profile and consent screen (design customer-identity-consent.md §4.3, §4.4, K5).
import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  ConsentRecordInput,
  ConsentRecordOutput,
  MyProfileData,
  ProfileUpdateInput,
  ProfileUpdateOutput,
} from '../../../shared/api/generated/my_profile';
import { lang, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import './profile.css';

const t = screenT('customer.profile', table);

const CHANNELS = ['SMS', 'EMAIL', 'PUSH'] as const;
type Channel = (typeof CHANNELS)[number];

export interface ProfileScreenProps {
  /** Injected so tests mock at the fetch level; defaults to `new ApiClient()`. */
  client?: ApiClient;
}

export function ProfileScreen(props: ProfileScreenProps = {}): JSX.Element {
  const api = props.client ?? new ApiClient();

  const [loading, setLoading] = createSignal(true);
  const [signedOut, setSignedOut] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [profile, setProfile] = createSignal<MyProfileData | null>(null);

  const [displayName, setDisplayName] = createSignal('');
  const [preferredLanguage, setPreferredLanguage] = createSignal('th');
  const [origName, setOrigName] = createSignal('');
  const [origLang, setOrigLang] = createSignal('th');

  const [saving, setSaving] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const [consents, setConsents] = createSignal<Record<Channel, boolean>>({
    SMS: false,
    EMAIL: false,
    PUSH: false,
  });
  const [consentBusy, setConsentBusy] = createSignal<Record<Channel, boolean>>({
    SMS: false,
    EMAIL: false,
    PUSH: false,
  });
  const [termsVersion, setTermsVersion] = createSignal<number | null>(null);

  const profileWrite = keyedWrite<ProfileUpdateInput, ProfileUpdateOutput>(
    api,
    'PUT',
    '/api/app/profile',
  );
  const consentWrites: Record<Channel, ReturnType<typeof keyedWrite<ConsentRecordInput, ConsentRecordOutput>>> = {
    SMS: keyedWrite<ConsentRecordInput, ConsentRecordOutput>(api, 'POST', '/api/app/consents'),
    EMAIL: keyedWrite<ConsentRecordInput, ConsentRecordOutput>(api, 'POST', '/api/app/consents'),
    PUSH: keyedWrite<ConsentRecordInput, ConsentRecordOutput>(api, 'POST', '/api/app/consents'),
  };

  const load = async () => {
    setLoading(true);
    setSignedOut(false);
    setLoadError(null);
    try {
      const res = await api.view<MyProfileData>('my_profile');
      const data = res.data;
      if (!data) return;
      setProfile(data);
      setDisplayName(data.displayName);
      setPreferredLanguage(data.preferredLanguage);
      setOrigName(data.displayName);
      setOrigLang(data.preferredLanguage);

      const nextConsents: Record<Channel, boolean> = { SMS: false, EMAIL: false, PUSH: false };
      const latestAt: Partial<Record<Channel, string>> = {};
      let latestTerms: { version: number; at: string } | null = null;

      for (const row of data.consents ?? []) {
        if (row.purpose === 'MARKETING' && (row.channel === 'SMS' || row.channel === 'EMAIL' || row.channel === 'PUSH')) {
          const prevAt = latestAt[row.channel];
          if (!prevAt || row.effectiveAt >= prevAt) {
            latestAt[row.channel] = row.effectiveAt;
            nextConsents[row.channel] = row.granted;
          }
        } else if (row.purpose === 'TERMS') {
          if (!latestTerms || row.effectiveAt >= latestTerms.at) {
            latestTerms = { version: row.policyVersion, at: row.effectiveAt };
          }
        }
      }
      setConsents(nextConsents);
      setTermsVersion(latestTerms ? latestTerms.version : null);
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        setSignedOut(true);
      } else {
        setLoadError(wordedError(err));
      }
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void load();
  });

  const canSave = () => {
    if (saving()) return false;
    if (!displayName().trim()) return false;
    return displayName() !== origName() || preferredLanguage() !== origLang();
  };

  const handleSave = async (e: Event) => {
    e.preventDefault();
    if (!canSave()) return;
    setSaving(true);
    setSaved(false);
    setError(null);
    const nextName = displayName();
    const nextLang = preferredLanguage();
    try {
      const out = await profileWrite.send({
        displayName: nextName !== origName() ? nextName : null,
        preferredLanguage: nextLang !== origLang() ? nextLang : null,
      });
      setOrigName(out.displayName);
      setOrigLang(out.preferredLanguage);
      setDisplayName(out.displayName);
      setPreferredLanguage(out.preferredLanguage);
      setSaved(true);
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  };

  const handleConsentChange = async (channel: Channel, el: HTMLInputElement) => {
    if (consentBusy()[channel]) return;
    const prev = consents()[channel];
    const next = el.checked;
    setConsents((c) => ({ ...c, [channel]: next }));
    setConsentBusy((b) => ({ ...b, [channel]: true }));
    setError(null);
    try {
      await consentWrites[channel].send({
        purpose: 'MARKETING',
        channel,
        granted: next,
        lang: lang(),
      });
    } catch (err) {
      setConsents((c) => ({ ...c, [channel]: prev }));
      el.checked = prev;
      setError(wordedError(err));
    } finally {
      setConsentBusy((b) => ({ ...b, [channel]: false }));
    }
  };

  const channelLabel = (channel: Channel) => {
    if (channel === 'SMS') return t('channelSMS');
    if (channel === 'EMAIL') return t('channelEMAIL');
    return t('channelPUSH');
  };

  return (
    <div class="xc-profile">
      <h1>{t('title')}</h1>

      <Show when={loading()}>
        <p class="xc-profile-loading">{t('loading')}</p>
      </Show>

      <Show when={!loading() && signedOut()}>
        <div class="xc-profile-signin" data-testid="profile-signin">
          <p>{t('signInPrompt')}</p>
          <a href="/app/sign-in">{t('signIn')}</a>
        </div>
      </Show>

      <Show when={!loading() && !signedOut() && loadError()}>
        {(msg) => (
          <div class="xc-profile-load-error" role="alert">
            <p>{msg()}</p>
            <button type="button" data-action="retry" onClick={() => void load()}>
              {t('retry')}
            </button>
          </div>
        )}
      </Show>

      <Show when={!loading() && !signedOut() && !loadError() && profile()}>
        {(p) => (
          <div class="xc-profile-body">
            <Show when={error()}>
              {(msg) => (
                <p class="xc-profile-error" data-testid="profile-error" role="alert">
                  {msg()}
                </p>
              )}
            </Show>

            <Show when={saved()}>
              <p class="xc-profile-saved" data-testid="profile-saved">
                {t('saved')}
              </p>
            </Show>

            <form class="xc-profile-form" onSubmit={handleSave}>
              <div class="xc-profile-row">
                <span>{t('customerNo')}</span>
                <span>{p().customerNo}</span>
              </div>

              <label class="xc-profile-field">
                <span>{t('displayName')}</span>
                <input
                  type="text"
                  data-field="displayName"
                  value={displayName()}
                  onInput={(e) => {
                    setDisplayName(e.currentTarget.value);
                    setSaved(false);
                  }}
                />
              </label>

              <label class="xc-profile-field">
                <span>{t('preferredLanguage')}</span>
                <select
                  data-field="preferredLanguage"
                  value={preferredLanguage()}
                  onChange={(e) => {
                    setPreferredLanguage(e.currentTarget.value);
                    setSaved(false);
                  }}
                >
                  <option value="th">{t('langTh')}</option>
                  <option value="en">{t('langEn')}</option>
                  <option value="zh">{t('langZh')}</option>
                </select>
              </label>

              <button
                type="submit"
                data-action="save-profile"
                disabled={!canSave()}
              >
                {t('save')}
              </button>
            </form>

            <section class="xc-profile-consents">
              <h2>{t('marketingTitle')}</h2>
              <p>{t('marketingHint')}</p>
              <For each={CHANNELS}>
                {(channel) => (
                  <label class="xc-profile-consent-row">
                    <input
                      type="checkbox"
                      data-consent={channel}
                      checked={consents()[channel]}
                      disabled={consentBusy()[channel]}
                      onChange={(e) => void handleConsentChange(channel, e.currentTarget)}
                    />
                    <span>{channelLabel(channel)}</span>
                  </label>
                )}
              </For>
            </section>

            <Show when={termsVersion() !== null}>
              <p class="xc-profile-terms" data-testid="terms-version">
                {t('termsAccepted', { version: String(termsVersion()) })}
              </p>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
