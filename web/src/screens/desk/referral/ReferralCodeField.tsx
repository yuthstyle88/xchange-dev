import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { AttachReferralInput, AttachReferralOutput } from '../../../shared/api/generated/attach_referral';
import type { MeView } from '../../../shared/api/generated/me';
import { PermissionsUnavailable, permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import { can } from '../../staff/parts/can';
import * as texts from './i18n';
import './referral.css';

export interface ReferralCodeFieldProps {
  transactionId: string;
  api?: ApiClient;
}

const referralWriters = new WeakMap<
  ApiClient,
  ReturnType<typeof keyedWrite<AttachReferralInput, AttachReferralOutput>>
>();

function writerFor(api: ApiClient) {
  let w = referralWriters.get(api);
  if (!w) {
    w = keyedWrite<AttachReferralInput, AttachReferralOutput>(
      api,
      'POST',
      (b) => `/api/transactions/${b.id}/referral`,
    );
    referralWriters.set(api, w);
  }
  return w;
}

export function ReferralCodeField(props: ReferralCodeFieldProps): JSX.Element {
  const t = screenT('desk.referral', texts);
  const defaultApi = new ApiClient();
  const getApi = () => props.api ?? defaultApi;
  const [me, setMe] = createSignal<MeView | undefined>(undefined);
  const [meError, setMeError] = createSignal<unknown>(null);
  const [code, setCode] = createSignal('');
  const [submitting, setSubmitting] = createSignal(false);
  const [locked, setLocked] = createSignal(false);
  const [savedCode, setSavedCode] = createSignal<string | null>(null);
  const [error, setError] = createSignal<unknown>(null);

  const loadMe = async () => {
    try {
      const res = await getApi().view<MeView>('me');
      const meData =
        res && typeof res === 'object' && 'data' in res && res.data
          ? res.data
          : (res as unknown as MeView);
      setMe(meData);
      setMeError(null);
    } catch (err: unknown) {
      setMe(undefined);
      setMeError(err);
    }
  };

  onMount(() => {
    void loadMe();
  });

  const handleSubmit = async () => {
    if (!can(me(), 'ReferralAttach')) return;
    if (submitting() || locked()) return;
    const c = code().trim();
    if (!c) {
      setError('codeRequired');
      return;
    }
    if (!/^[A-Za-z0-9]{6,12}$/.test(c)) {
      setError('invalidCode');
      return;
    }

    setSubmitting(true);
    setError(null);

    const referralWrite = writerFor(getApi());
    try {
      await referralWrite.send({ id: props.transactionId, referralCode: c });
      setSavedCode(c);
      setLocked(true);
    } catch (err: unknown) {
      if (err instanceof ApiError && (err.status === 409 || err.code === 'referralAlreadyCaptured')) {
        setError('alreadyCaptured');
        setLocked(true);
      } else if (err instanceof ApiError && err.code === 'validationFailed' && err.field === 'referralCode') {
        setError('invalidCode');
      } else {
        setError(err);
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Show when={can(me(), 'ReferralAttach') || meError() !== null}>
      <div class="desk-referral-field">
        <label class="desk-referral-label">
          <span>{t('label')}</span>
          <div class="desk-referral-row">
            <input
              type="text"
              class="desk-referral-input"
              placeholder={t('placeholder')}
              value={code()}
              onInput={(e) => setCode(e.currentTarget.value)}
              disabled={locked() || submitting() || meError() !== null}
            />
            <button
              type="button"
              class="desk-referral-btn"
              onClick={() => void handleSubmit()}
              disabled={locked() || submitting() || meError() !== null}
              title={meError() !== null ? permissionsUnavailableText(meError()) : undefined}
            >
              {submitting() ? t('saving') : t('save')}
            </button>
          </div>
        </label>

        <PermissionsUnavailable error={meError()} onRetry={() => void loadMe()} />

        <Show when={error()}>
          <p class="desk-referral-error" role="alert">
            {error() === 'alreadyCaptured'
              ? t('alreadyCaptured')
              : error() === 'codeRequired'
                ? t('codeRequired')
                : error() === 'invalidCode'
                  ? t('invalidCode')
                  : typeof error() === 'string'
                    ? (error() as string)
                    : wordedError(error())}
          </p>
        </Show>

        <Show when={savedCode()}>
          <p class="desk-referral-success">
            {t('saved')} <strong class="desk-referral-code">{savedCode()}</strong>
          </p>
        </Show>
      </div>
    </Show>
  );
}