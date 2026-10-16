import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { wordedError } from '../../../shared/i18n/worded-error';
import type {
  ConsentTextsData,
  ConsentTextViewItem,
} from '../../../shared/api/generated/consent_texts';
import { screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './consent.css';

const t = screenT('admin.consent', table);

const DATA_TYPES = ['IDENTITY_VALUE', 'CONTACT', 'CLOSED_CUSTOMER'] as const;
type DataType = (typeof DATA_TYPES)[number];

interface RetentionRowProps {
  dataType: DataType;
  branch: () => string;
  client: ApiClient;
}

function RetentionRow(props: RetentionRowProps): JSX.Element {
  const saveRetention = keyedWrite(
    props.client,
    'PUT',
    `/api/retention-policies/${props.dataType}`,
  );
  const [days, setDays] = createSignal('');
  const [error, setError] = createSignal<unknown>(null);
  const [busy, setBusy] = createSignal(false);

  const handleSave = async () => {
    setError(null);
    const raw = days().trim();
    let periodDays: number | null;

    if (raw === '') {
      periodDays = null;
    } else if (/^[1-9]\d*$/.test(raw)) {
      periodDays = parseInt(raw, 10);
    } else {
      const msg = t('periodInvalid');
      setError(msg);
      toast(msg);
      return;
    }

    setBusy(true);
    try {
      await saveRetention.send({
        periodDays,
        branch: props.branch(),
      });
      toast(t('save'));
    } catch (err: unknown) {
      setError(err);
      toast(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div data-retention={props.dataType} class="retention-row">
      <div class="retention-label">
        {t(`data${props.dataType}` as keyof typeof table.th)}
      </div>
      <label>
        {t('periodDays')}
        <input
          type="text"
          value={days()}
          onInput={(e) => setDays(e.currentTarget.value)}
          onChange={(e) => setDays(e.currentTarget.value)}
          disabled={busy()}
        />
      </label>
      <Show when={error()}>
        <div class="error">{typeof error() === 'string' ? (error() as string) : wordedError(error())}</div>
      </Show>
      <button
        type="button"
        onClick={handleSave}
        disabled={busy()}
      >
        {t('save')}
      </button>
    </div>
  );
}

export function Consent(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();
  const publishWrite = keyedWrite(client, 'POST', '/api/consent-texts');

  const [texts, setTexts] = createSignal<ConsentTextViewItem[]>([]);
  const [purpose, setPurpose] = createSignal('MARKETING');
  const [lang, setLang] = createSignal('th');
  const [body, setBody] = createSignal('');
  const [publishError, setPublishError] = createSignal<unknown>(null);
  const [publishing, setPublishing] = createSignal(false);

  let loadSeq = 0;
  let loadedBranch: string | undefined;

  const loadTexts = async (branchName: string) => {
    const seq = ++loadSeq;
    try {
      const res = await client.view<ConsentTextsData>('consent_texts', { branch: branchName });
      if (seq !== loadSeq) return;
      if (res.data?.texts) {
        setTexts(res.data.texts);
      }
    } catch (err: unknown) {
      if (seq !== loadSeq) return;
      setTexts([]);
      toast(wordedError(err));
    }
  };

  createEffect(() => {
    const b = props.branch();
    const s = props.shown ? props.shown() : true;
    if (b !== loadedBranch) {
      loadedBranch = b;
      setTexts([]);
    }
    if (s) {
      loadTexts(b);
    }
  });

  const currentTexts = () => {
    const list = texts();
    const newest = new Map<string, ConsentTextViewItem>();
    for (const item of list) {
      const k = `${item.purpose}:${item.lang}`;
      const existing = newest.get(k);
      if (!existing || existing.version < item.version) {
        newest.set(k, item);
      }
    }
    return [...newest.values()].sort((a, b) =>
      `${a.purpose}:${a.lang}`.localeCompare(`${b.purpose}:${b.lang}`),
    );
  };

  const handlePublish = async () => {
    setPublishError(null);
    const textBody = body();
    if (!textBody.trim()) {
      const msg = t('bodyRequired');
      setPublishError(msg);
      toast(msg);
      return;
    }

    setPublishing(true);
    try {
      await publishWrite.send({
        purpose: purpose(),
        lang: lang(),
        body: textBody,
        branch: props.branch(),
      });
      setBody('');
      await loadTexts(props.branch());
    } catch (err: unknown) {
      setPublishError(err);
      toast(wordedError(err));
    } finally {
      setPublishing(false);
    }
  };

  return (
    <div class="admin-consent-screen">
      <section class="consent-section">
        <div class="consent-texts-list">
          <For each={currentTexts()}>
            {(item) => (
              <div data-consent-text={`${item.purpose}:${item.lang}`} class="consent-text-item">
                <div class="consent-text-meta">
                  <span class="purpose">{item.purpose}</span>
                  <span class="lang">{item.lang}</span>
                  <span class="version">v{item.version}</span>
                </div>
                <div class="consent-text-body">{item.body}</div>
              </div>
            )}
          </For>
        </div>

        <div class="consent-publish-form">
          <label>
            {t('purpose')}
            <select
              value={purpose()}
              onInput={(e) => setPurpose(e.currentTarget.value)}
              onChange={(e) => setPurpose(e.currentTarget.value)}
              disabled={publishing()}
            >
              <option value="MARKETING">MARKETING</option>
              <option value="TERMS">TERMS</option>
            </select>
          </label>

          <label>
            {t('language')}
            <select
              value={lang()}
              onInput={(e) => setLang(e.currentTarget.value)}
              onChange={(e) => setLang(e.currentTarget.value)}
              disabled={publishing()}
            >
              <option value="en">en</option>
              <option value="th">th</option>
              <option value="zh">zh</option>
            </select>
          </label>

          <label>
            {t('body')}
            <textarea
              value={body()}
              onInput={(e) => setBody(e.currentTarget.value)}
              onChange={(e) => setBody(e.currentTarget.value)}
              disabled={publishing()}
            />
          </label>

          <Show when={publishError()}>
            <div class="error">
              {typeof publishError() === 'string'
                ? (publishError() as string)
                : wordedError(publishError())}
            </div>
          </Show>

          <button
            type="button"
            class="primary"
            onClick={handlePublish}
            disabled={publishing()}
          >
            {t('publish')}
          </button>
        </div>
      </section>

      <section class="retention-section">
        <div class="retention-list">
          <For each={DATA_TYPES}>
            {(dataType) => (
              <RetentionRow
                dataType={dataType}
                branch={props.branch}
                client={client}
              />
            )}
          </For>
        </div>
      </section>
    </div>
  );
}
