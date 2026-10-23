import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  MaskedMemberSample,
  SegmentPreviewDto,
  SegmentPreviewFilter,
} from '../../../shared/api/generated/segment_preview';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError, type FieldMessage } from '../../../shared/i18n/field-error';
import { isServerFailure } from '../../../shared/i18n/worded-error';
import { checkDecimal, MONEY } from '../../../shared/ui/bounded';
import { MoneyText } from '../../../shared/ui/MoneyText';


import type { AdminMenuProps } from '../menus';
import * as table from './i18n';

const t = screenT('admin.members', table);

interface DefineBody {
  name: string;
  rule: Record<string, unknown>;
}

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

function memberError(err: unknown): FieldMessage {
  const r = fieldError(err, {
    name: t('fieldName'),
    'rule.recencyDays': t('fieldRecencyDays'),
    'rule.recency_days': t('fieldRecencyDays'),
    'rule.frequency': t('fieldFrequency'),
    'rule.valueThb': t('fieldValueThb'),
    'rule.value_thb': t('fieldValueThb'),
    value_thb: t('fieldValueThb'),
    valueThb: t('fieldValueThb'),
  });
  if (r.field === 'rule.recency_days') {
    return { ...r, field: 'rule.recencyDays' };
  }
  if (r.field === 'rule.value_thb' || r.field === 'value_thb' || r.field === 'valueThb') {
    return { ...r, field: 'rule.valueThb' };
  }
  if (!r.field && err instanceof ApiError && !isServerFailure(err)) {
    const mapped = ERROR_KEYS[err.code];
    if (mapped) {
      return { message: t(mapped) };
    }
  }
  return r;
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
  const [error, setError] = createSignal<FieldMessage | null>(null);

  const [definition, setDefinition] = createSignal<SegmentDefineOutput | null>(null);
  const [snapshotInfo, setSnapshotInfo] = createSignal<SegmentSnapshotOutput | null>(null);
  const [preview, setPreview] = createSignal<SegmentPreviewDto | null>(null);

  const defineWrite = keyedWrite<DefineBody, SegmentDefineOutput>(api, 'POST', '/api/segments');
  const snapshotWrite = keyedWrite<{ id: string }, SegmentSnapshotOutput>(
    api,
    'POST',
    (b) => `/api/segments/${b.id}/snapshots`,
  );

  const fieldMessage = (field: string): string | null => {
    const cur = error();
    return cur?.field === field ? cur.message : null;
  };

  async function handleDefine(e?: Event) {
    e?.preventDefault();
    if (defining()) return;
    setDefining(true);
    setError(null);

    try {
      const nameTrimmed = name().trim();
      if (nameTrimmed === '' || nameTrimmed.length > 80) {
        setError(
          memberError(
            new ApiError({ status: 400, code: 'validationFailed', field: 'name', message: '' }),
          ),
        );
        return;
      }

      const rule: Record<string, unknown> = {};
      const recencyTrimmed = recencyDays().trim();
      if (recencyTrimmed !== '') {
        if (!/^\d+$/.test(recencyTrimmed)) {
          setError({ message: t('error.validationFailed') });
          return;
        }
        const n = Number.parseInt(recencyTrimmed, 10);
        if (n > 4294967295) {
          setError(
            memberError(
              new ApiError({
                status: 400,
                code: 'validationFailed',
                field: 'rule.recencyDays',
                message: '',
              }),
            ),
          );
          return;
        }
        rule.recency_days = n;
      }
      const freqTrimmed = frequency().trim();
      if (freqTrimmed !== '') {
        if (!/^\d+$/.test(freqTrimmed)) {
          setError({ message: t('error.validationFailed') });
          return;
        }
        const n = Number.parseInt(freqTrimmed, 10);
        if (n > 4294967295) {
          setError(
            memberError(
              new ApiError({
                status: 400,
                code: 'validationFailed',
                field: 'rule.frequency',
                message: '',
              }),
            ),
          );
          return;
        }
        rule.frequency = n;
      }
      const valTrimmed = valueThb().trim();
      if (valTrimmed !== '') {
        const checked = checkDecimal(valTrimmed, MONEY);
        if (!checked.ok) {
          setError(
            memberError(
              new ApiError({
                status: 400,
                code: 'validationFailed',
                field: 'rule.valueThb',
                message: '',
              }),
            ),
          );
          return;
        }
        rule.value_thb = checked.text;
      }
      if (tier().trim() !== '') {
        rule.tier = tier().trim();
      }

      const body = {
        name: nameTrimmed,
        rule,
      };

      const res = await defineWrite.send(body);
      setDefinition(res);
      setSnapshotInfo(null);
      setPreview(null);
    } catch (err) {
      setError(memberError(err));
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
    if (snapshotting()) return;
    const def = definition();
    if (!def) return;
    setSnapshotting(true);
    setError(null);

    try {
      const body = { id: def.id };
      const res = await snapshotWrite.send(body);
      setSnapshotInfo(res);
      setPreview(null);
      await loadPreview(res.id);
    } catch (err) {
      setError(memberError(err));
    } finally {
      setSnapshotting(false);
    }
  }



  return (
    <div class="xc-campaigns-screen">
      <Show when={error() && !error()!.field ? error()!.message : null}>
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
        <form noValidate onSubmit={(e) => void handleDefine(e)}>
          <div class="xc-form-grid">
            <div class="xc-form-group">
              <label for="segment-name">{t('fieldName')}</label>
              <input
                id="segment-name"
                name="name"
                data-field="name"
                type="text"
                maxLength={80}
                required
                class="xc-input"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
              />
              <Show when={fieldMessage('name')}>
                {(msg) => <span data-field-error="name">{msg()}</span>}
              </Show>
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
              <Show when={fieldMessage('rule.recencyDays')}>
                {(msg) => <span data-field-error="rule.recencyDays">{msg()}</span>}
              </Show>
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
              <Show when={fieldMessage('rule.frequency')}>
                {(msg) => <span data-field-error="rule.frequency">{msg()}</span>}
              </Show>
            </div>

            <div class="xc-form-group">
              <label for="segment-value-thb">{t('fieldValueThb')}</label>
              <input
                id="segment-value-thb"
                name="value_thb"
                data-field="value_thb"
                type="text"
                max="100000000"
                class="xc-input"
                value={valueThb()}
                onInput={(e) => setValueThb(e.currentTarget.value)}
              />
              <Show when={fieldMessage('rule.valueThb')}>
                {(msg) => <span data-field-error="rule.valueThb">{msg()}</span>}
              </Show>
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
            <button
              type="submit"
              data-action="define"
              class="xc-btn-primary"
              disabled={defining()}
            >
              {defining() ? t('defining') : t('defineBtn')}
            </button>
            <button
              type="button"
              data-action="snapshot"
              class="xc-btn-secondary"
              disabled={snapshotting() || !definition()}
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


