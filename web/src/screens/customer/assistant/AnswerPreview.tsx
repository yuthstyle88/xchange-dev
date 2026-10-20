// C12: what the assistant understood — the intent's preview to confirm as a planner draft, or the link its kind
// leads to (design customer-app.md P11 `:71`; section-b-gaps.md S4; BE contract #3407: the link is derived on the
// client from `intent.kind`, the back end sends no URL).
import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { AssistantAskAnswerDto } from '../../../shared/api/generated/assistant_ask';
import type { PlannerDraftSaveInput } from '../../../shared/api/generated/planner_draft_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { en, th, zh } from './i18n';

const t = screenT('customer.assistant', { th, en, zh });

/** The intent kinds #3407 sends; the generated `AssistantIntentDto.kind` is a plain string. */
export type AssistantIntentKind = 'QUOTE' | 'QUEUE' | 'ALERT' | 'BRANCH' | 'UNKNOWN';

export interface AnswerPreviewProps {
  answer: AssistantAskAnswerDto;
  /** The screen's client; the planner-draft write goes through `keyedWrite`. */
  api: ApiClient;
  /** The screen's branch, used when the intent names none. */
  branch?: string;
}

const KIND_LINKS: Partial<
  Record<AssistantIntentKind, { href: string; labelKey: 'linkQueue' | 'linkAlerts' | 'linkBranches' }>
> = {
  QUEUE: { href: '/app/journey', labelKey: 'linkQueue' },
  ALERT: { href: '/app/rate-alerts', labelKey: 'linkAlerts' },
  BRANCH: { href: '/app/branches', labelKey: 'linkBranches' },
};

export function AnswerPreview(props: AnswerPreviewProps): JSX.Element {
  const writer = keyedWrite<PlannerDraftSaveInput, unknown>(
    props.api,
    'POST',
    '/api/app/planner-drafts',
  );

  const [inFlight, setInFlight] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let currentAnswer = props.answer;
  createEffect(() => {
    const next = props.answer;
    if (next !== currentAnswer) {
      currentAnswer = next;
      setInFlight(false);
      setSaved(false);
      setError(null);
      writer.reset();
    }
  });

  const draftInput = (): PlannerDraftSaveInput | null => {
    const intent = props.answer.intent;
    if (!intent || intent.kind !== 'QUOTE' || !props.answer.preview) return null;
    if (!intent.currency || !intent.customerSide) return null;
    const amount =
      intent.quantity !== undefined
        ? { quantity: intent.quantity }
        : intent.budgetThb !== undefined
          ? { budgetThb: intent.budgetThb }
          : null;
    if (!amount) return null;
    const branch = intent.branch ?? props.branch;
    return {
      ...(branch ? { branch } : {}),
      currency: intent.currency,
      customerSide: intent.customerSide,
      amount,
    };
  };

  const handleConfirm = async () => {
    const body = draftInput();
    if (!body || inFlight() || saved()) return;
    const target = props.answer;
    setInFlight(true);
    setError(null);
    try {
      await writer.send(body);
      if (props.answer !== target) return;
      setSaved(true);
    } catch {
      if (props.answer !== target) return;
      setError(t('previewSaveFailed'));
    } finally {
      if (props.answer === target) {
        setInFlight(false);
      }
    }
  };

  const directLink = () => {
    const kind = props.answer.intent?.kind;
    return kind && kind in KIND_LINKS ? KIND_LINKS[kind as AssistantIntentKind] : undefined;
  };

  return (
    <>
      <Show when={directLink()}>
        {(link) => (
          <div class="assistant-preview">
            <a class="assistant-preview-link" href={link().href}>
              {t(link().labelKey)}
            </a>
          </div>
        )}
      </Show>

      <Show when={draftInput() && props.answer.preview}>
        {(preview) => (
          <div class="assistant-preview">
            <div class="assistant-preview-title">{t('previewTitle')}</div>
            <div class="assistant-preview-thb" data-testid="preview-thb">
              <MoneyText value={preview().thb} currency="THB" />
            </div>
            <Show when={preview().stale}>
              <div class="assistant-preview-stale" data-testid="preview-stale">
                {t('previewStale')}
              </div>
            </Show>
            <Show when={!saved()}>
              <button
                type="button"
                class="assistant-preview-confirm"
                data-action="confirm-plan"
                disabled={inFlight()}
                onClick={() => void handleConfirm()}
              >
                {t('previewConfirm')}
              </button>
            </Show>
            <Show when={saved()}>
              <div class="assistant-preview-saved" data-testid="preview-saved">
                {t('previewSaved')}
              </div>
              <a class="assistant-preview-link" href="/app/journey">
                {t('linkPlans')}
              </a>
            </Show>
            <Show when={!saved() && error()}>
              {(msg) => (
                <div class="assistant-preview-error" data-testid="preview-error">
                  {msg()}
                </div>
              )}
            </Show>
          </div>
        )}
      </Show>
    </>
  );
}

