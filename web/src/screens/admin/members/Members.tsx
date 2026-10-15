import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  MaskedMemberSample,
  SegmentPreviewDto,
  SegmentPreviewFilter,
} from '../../../shared/api/generated/segment_preview';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { MoneyText } from '../../../shared/ui/MoneyText';


import type { AdminMenuProps } from '../menus';
import * as table from './i18n';

const t = screenT('admin.members', table);

interface SegmentDefineOutput {
  id: string;
  version: number;
}

interface SegmentSnapshotOutput {
  id: string;
  memberCount: number;
}

const ERROR_KEYS: Record<string, keyof typeof table.th> = {
  validationFailed: 'error.validationFailed',
  outOfScope: 'error.outOfScope',
  roleNotAllowed: 'error.roleNotAllowed',
};

function memberError(err: unknown): string {
  if (err instanceof ApiError) {
    if (isServerFailure(err)) {
      return wordedError(err);
    }
    const mapped = ERROR_KEYS[err.code];
    if (mapped) {
      return t(mapped);
    }
  }
  return wordedError(err);
}

export function Members(_props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [name, setName] = createSignal('');
  const [recencyDays, setRecencyDays] = createSignal('30');
  const [frequency, setFrequency] = createSignal('');
  const [valueThb, setValueThb] = createSignal('');
  const [tier, setTier] = createSignal('');

  const [defining, setDefining] = createSignal(false);
  const [snapshotting, setSnapshotting] = createSignal(false);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);

  const [definition, setDefinition] = createSignal<SegmentDefineOutput | null>(null);
  const [snapshotInfo, setSnapshotInfo] = createSignal<SegmentSnapshotOutput | null>(null);
  const [preview, setPreview] = createSignal<SegmentPreviewDto | null>(null);

  let pendingActionKey: { action: string; payload: string; key: string } | null = null;

  function getIdempotencyKey(action: string, body: unknown): string {
    const payload = JSON.stringify(body);
    if (pendingActionKey && pendingActionKey.action === action && pendingActionKey.payload === payload) {
      return pendingActionKey.key;
    }
    const key =
      typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
        ? crypto.randomUUID()
        : `idem-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    pendingActionKey = { action, payload, key };
    return key;
  }

  function clearPendingKey() {
    pendingActionKey = null;
  }

  async function handleDefine(e?: Event) {
    e?.preventDefault();
    setDefining(true);
    setErrorMsg(null);

    try {
      const rule: Record<string, unknown> = {};
      const parsedRecency = Number.parseInt(recencyDays().trim(), 10);
      if (!Number.isNaN(parsedRecency)) {
        rule.recency_days = parsedRecency;
      }
      const parsedFreq = Number.parseInt(frequency().trim(), 10);
      if (frequency().trim() !== '' && !Number.isNaN(parsedFreq)) {
        rule.frequency = parsedFreq;
      }
      if (valueThb().trim() !== '') {
        rule.value_thb = valueThb().trim();
      }
      if (tier().trim() !== '') {
        rule.tier = tier().trim();
      }

      const body = {
        name: name().trim(),
        rule,
      };

      const key = getIdempotencyKey('segment_define', body);
      const res = await api.command<typeof body, SegmentDefineOutput>(
        'POST',
        '/api/segments',
        body,
        key,
      );
      clearPendingKey();
      setDefinition(res);
    } catch (err) {
      setErrorMsg(memberError(err));
    } finally {
      setDefining(false);
    }
  }

  async function loadPreview(snapshotId: string) {
    const res = await api.view<SegmentPreviewDto, SegmentPreviewFilter>('segment_preview', {
      filter: { id: snapshotId },
    });
    if (res.changed && res.data) {
      setPreview(res.data);
    }
  }

  async function handleSnapshot() {
    const def = definition();
    if (!def) return;
    setSnapshotting(true);
    setErrorMsg(null);

    try {
      const body = { id: def.id };
      const key = getIdempotencyKey('segment_snapshot', body);
      const res = await api.command<typeof body, SegmentSnapshotOutput>(
        'POST',
        `/api/segments/${def.id}/snapshots`,
        body,
        key,
      );
      clearPendingKey();
      setSnapshotInfo(res);
      await loadPreview(res.id);
    } catch (err) {
      setErrorMsg(memberError(err));
    } finally {
      setSnapshotting(false);
    }
  }



  return (
    <div class="xc-campaigns-screen">
      <Show when={errorMsg()}>
        {(msg) => (
          <div class="xc-campaigns-error" role="alert">
            <span>{msg()}</span>
          </div>
        )}
      </Show>

      <section class="xc-campaign-detail">
        <div class="xc-detail-header">
          <h3>{t('defineTitle')}</h3>
        </div>
        <form onSubmit={(e) => void handleDefine(e)}>
          <div class="xc-form-grid">
            <div class="xc-form-group">
              <label for="segment-name">{t('fieldName')}</label>
              <input
                id="segment-name"
                name="name"
                data-field="name"
                type="text"
                class="xc-input"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label for="segment-recency-days">{t('fieldRecencyDays')}</label>
              <input
                id="segment-recency-days"
                name="recency_days"
                data-field="recency_days"
                type="number"
                class="xc-input"
                value={recencyDays()}
                onInput={(e) => setRecencyDays(e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label for="segment-frequency">{t('fieldFrequency')}</label>
              <input
                id="segment-frequency"
                name="frequency"
                data-field="frequency"
                type="number"
                class="xc-input"
                value={frequency()}
                onInput={(e) => setFrequency(e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label for="segment-value-thb">{t('fieldValueThb')}</label>
              <input
                id="segment-value-thb"
                name="value_thb"
                data-field="value_thb"
                type="text"
                class="xc-input"
                value={valueThb()}
                onInput={(e) => setValueThb(e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label for="segment-tier">{t('fieldTier')}</label>
              <input
                id="segment-tier"
                name="tier"
                data-field="tier"
                type="text"
                class="xc-input"
                value={tier()}
                onInput={(e) => setTier(e.currentTarget.value)}
              />
            </div>
          </div>

          <div class="xc-detail-actions">
            <button type="submit" data-action="define" class="xc-btn-primary">
              {defining() ? t('defining') : t('defineBtn')}
            </button>
            <button
              type="button"
              data-action="snapshot"
              class="xc-btn-secondary"
              onClick={() => void handleSnapshot()}
            >
              {snapshotting() ? t('snapshotting') : t('snapshotBtn')}
            </button>
          </div>
        </form>
      </section>

      <Show when={snapshotInfo()}>
        {(snap) => (
          <section class="xc-campaign-detail">
            <div class="xc-detail-header">
              <h3>{t('snapshotTitle')}</h3>
            </div>
            <div>
              <p>
                {t('snapshotMemberCount')}: <strong>{snap().memberCount}</strong>
              </p>
            </div>
          </section>
        )}
      </Show>

      <Show when={preview()}>
        {(prev) => (
          <section class="xc-campaign-detail">
            <div class="xc-detail-header">
              <h3>{t('previewTitle')}</h3>
            </div>
            <div class="xc-form-grid">
              <div class="xc-form-group">
                <span>{t('previewMemberCount')}</span>
                <strong>{prev().memberCount}</strong>
              </div>
              <div class="xc-form-group">
                <span>{t('avgRecencyDays')}</span>
                <strong>{prev().distribution.avgRecencyDays ?? t('dash')}</strong>
              </div>
              <div class="xc-form-group">
                <span>{t('avgFrequency')}</span>
                <strong>{prev().distribution.avgFrequency}</strong>
              </div>
              <div class="xc-form-group">
                <span>{t('totalValueThb')}</span>
                <strong>
                  <MoneyText value={prev().distribution.totalValueThb} currency="THB" />
                </strong>
              </div>
            </div>

            <div class="xc-campaigns-table-container">
              <table class="xc-campaigns-table">
                <thead>
                  <tr>
                    <th>{t('sampleHeaderName')}</th>
                    <th class="text-right">{t('sampleHeaderRecency')}</th>
                    <th class="text-right">{t('sampleHeaderFrequency')}</th>
                    <th class="text-right">{t('sampleHeaderValueThb')}</th>
                    <th>{t('sampleHeaderTier')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={prev().sample}>
                    {(row: MaskedMemberSample) => (
                      <tr>
                        <td>{row.maskedName}</td>
                        <td class="text-right">{row.metrics.recencyDays ?? t('dash')}</td>
                        <td class="text-right">{row.metrics.frequency}</td>
                        <td class="text-right">
                          <MoneyText value={row.metrics.valueThb} currency="THB" />
                        </td>
                        <td>{row.metrics.tierCode ?? t('dash')}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </section>
        )}
      </Show>
    </div>
  );
}


