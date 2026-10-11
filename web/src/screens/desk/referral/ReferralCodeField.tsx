import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AttachReferralInput, AttachReferralOutput } from '../../../shared/api/generated/attach_referral';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { can } from '../../staff/parts/can';
import * as texts from './i18n';
import './referral.css';

export interface ReferralCodeFieldProps {
  transactionId: string;
  api?: ApiClient;
}

export function ReferralCodeField(props: ReferralCodeFieldProps): JSX.Element {
  const t = screenT('desk.referral', texts);
  const [me, setMe] = createSignal<StaffMeView | undefined>(undefined);
  const [code, setCode] = createSignal('');
  const [submitting, setSubmitting] = createSignal(false);
  const [locked, setLocked] = createSignal(false);
  const [savedCode, setSavedCode] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  onMount(async () => {
    const api = props.api ?? new ApiClient();
    try {
      const res = await api.view<StaffMeView>('staff_me');
      const meData =
        res && typeof res === 'object' && 'data' in res && res.data
          ? res.data
          : (res as unknown as StaffMeView);
      setMe(meData);
    } catch {
      // If staff_me fails, the field is hidden.
    }
  });

  const handleSubmit = async () => {
    if (!can(me(), 'ReferralAttach')) return;
    if (submitting() || locked()) return;
    const c = code();
    if (!c.trim()) return;

    setSubmitting(true);
    setError(null);

    const api = props.api ?? new ApiClient();
    try {
      const key = crypto.randomUUID();
      await api.command<AttachReferralInput, AttachReferralOutput>(
        'POST',
        `/api/transactions/${props.transactionId}/referral`,
        { id: props.transactionId, referralCode: c },
        key,
      );
      setSavedCode(c);
      setLocked(true);
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        if (err.status === 409 || err.code === 'referralAlreadyCaptured') {
          setError(t('alreadyCaptured'));
          setLocked(true);
        } else {
          setError(err.message || t('invalidCode'));
        }
      } else if (err instanceof Error) {
        setError(err.message);
      } else {
        setError(String(err));
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Show when={can(me(), 'ReferralAttach')}>
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
              disabled={locked() || submitting()}
            />
            <button
              type="button"
              class="desk-referral-btn"
              onClick={() => void handleSubmit()}
              disabled={locked() || submitting()}
            >
              {submitting() ? t('saving') : t('save')}
            </button>
          </div>
        </label>

        <Show when={error()}>
          <p class="desk-referral-error" role="alert">
            {error()}
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