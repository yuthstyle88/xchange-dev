// #1349 V3-documents (design §A.3): one exchange's documents, for the owner's `/documents` and the staff's
// `/staff/documents`. The page reads its view once; the audience decides the links and what a refusal does.
import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../shared/api/client';
import type { DocumentsData, DocumentsFilter } from '../../shared/api/generated/documents';
import { screenLookup, screenT, translator } from '../../shared/i18n';
import type { Audience } from './audiences';
import * as table from './i18n';
import { refusalText } from './refusal';
import './documents.css';

const t = screenT('documents', table);
const tr = translator(screenLookup);

export interface DocumentsScreenProps {
  api: ApiClient;
  audience: Audience;
  reference: string;
  branch: string;
  replace: (url: string) => void;
}

export function DocumentsScreen(props: DocumentsScreenProps): JSX.Element {
  const [data, setData] = createSignal<DocumentsData | null>(null);
  const [refused, setRefused] = createSignal<string | null>(null);

  onMount(async () => {
    const { api, audience, reference, branch } = props;
    try {
      const answer = await api.view<DocumentsData, DocumentsFilter>(audience.view, {
        branch: audience.sendsBranch && branch !== '' ? branch : undefined,
        filter: { ref: reference },
      });
      setData(answer.data ?? null);
    } catch (err: unknown) {
      if (audience.refusalGoesTo !== null) {
        props.replace(audience.refusalGoesTo);
        return;
      }
      setRefused(refusalText(err, tr));
    }
  });

  const label = (kind: DocumentsData['links'][number]['kind']) => t(`kind.${kind}`);

  return (
    <div class="documents-screen">
      <Show when={props.audience.showDownload} fallback={<h2 class="documents-title">{data()?.ref ?? ''}</h2>}>
        <h2 class="documents-title">{t('title')}</h2>
        <Show when={data()}>{(d) => <p class="documents-ref">{t('refLine', { ref: d().ref })}</p>}</Show>
      </Show>

      <Show when={refused()}>
        <p class="documents-refusal" role="alert">
          {refused()}
        </p>
      </Show>

      <Show when={data()}>
        {(d) => (
          <Show
            when={props.audience.showDownload}
            fallback={
              <For each={d().links}>
                {(link) => (
                  <p data-kind={link.kind}>
                    <a href={link.href}>{label(link.kind)}</a>
                  </p>
                )}
              </For>
            }
          >
            <ul class="documents-list">
              <For each={d().links}>
                {(link) => (
                  <li data-kind={link.kind}>
                    {label(link.kind)} —{' '}
                    <a href={link.href} target="_blank" rel="noopener">
                      {t('open')}
                    </a>{' '}
                    ·{' '}
                    <a href={link.href} download={link.download ?? ''}>
                      {t('download', { name: link.download ?? '' })}
                    </a>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        )}
      </Show>

      <a class="documents-back" href={props.audience.backHref(props.branch)}>
        {t(props.audience.backText)}
      </a>
    </div>
  );
}
