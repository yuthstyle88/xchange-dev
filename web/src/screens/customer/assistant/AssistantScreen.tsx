import {
  createSignal,
  For,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  AssistantAskAnswerDto,
  AssistantAskInput,
} from '../../../shared/api/generated/assistant_ask';
import type {
  AssistantMessageDto,
  MyAssistantConversationData,
  MyAssistantConversationFilter,
} from '../../../shared/api/generated/my_assistant_conversation';
import { lang, locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { th, en, zh } from './i18n';
import './assistant.css';

const t = screenT('customer.assistant', { th, en, zh });

export interface AssistantScreenProps {
  api?: ApiClient;
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  initialConversationId?: string;
  branch?: string;
  search?: string;
}

const AMOUNT_REGEX =
  /(?:([A-Z]{3})\s+(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)|(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)\s+([A-Z]{3}))/g;

export function renderTextWithMoney(text: string): JSX.Element[] {
  const elements: JSX.Element[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  AMOUNT_REGEX.lastIndex = 0;
  while ((match = AMOUNT_REGEX.exec(text)) !== null) {
    if (match.index > lastIndex) {
      elements.push(<span>{text.slice(lastIndex, match.index)}</span>);
    }

    let currency = '';
    let rawVal = '';

    if (match[1] && match[2]) {
      currency = match[1];
      rawVal = match[2];
    } else if (match[3] && match[4]) {
      rawVal = match[3];
      currency = match[4];
    }

    const cleanVal = rawVal.replace(/,/g, '');
    elements.push(<MoneyText value={cleanVal} currency={currency} />);
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    elements.push(<span>{text.slice(lastIndex)}</span>);
  }

  return elements.length > 0 ? elements : [<span>{text}</span>];
}

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleTimeString(locale(), {
      hour: '2-digit',
      minute: '2-digit',
      timeZone: 'Asia/Bangkok',
    });
  } catch {
    return iso;
  }
}

function parseSearch(searchStr: string): { branch?: string; conversationId?: string } {
  try {
    const params = new URLSearchParams(searchStr);
    const branch = params.get('branch') || undefined;
    const conversationId = params.get('conversation') || params.get('conversationId') || undefined;
    return { branch, conversationId };
  } catch {
    return {};
  }
}

export function AssistantScreen(props: AssistantScreenProps): JSX.Element {
  const client = props.api ?? (props.fetch ? new ApiClient(props.fetch) : new ApiClient());
  const askWrite = keyedWrite<AssistantAskInput, AssistantAskAnswerDto>(
    client,
    'POST',
    '/api/app/assistant/messages',
  );

  const searchParams = () =>
    parseSearch(
      props.search ??
        (typeof window !== 'undefined' && window.location ? window.location.search : ''),
    );

  const effectiveBranch = () => props.branch ?? searchParams().branch;

  const [conversationId, setConversationId] = createSignal<string | null>(
    props.initialConversationId ?? searchParams().conversationId ?? null,
  );
  const [messages, setMessages] = createSignal<AssistantMessageDto[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [inputText, setInputText] = createSignal('');
  const [sending, setSending] = createSignal(false);
  const [sendError, setSendError] = createSignal<string | null>(null);

  const loadConversation = async (targetId?: string) => {
    const id = targetId ?? conversationId();
    if (!id) {
      setLoading(false);
      setLoadError(null);
      return;
    }

    setLoading(true);
    setLoadError(null);

    try {
      const res = await client.view<MyAssistantConversationData, MyAssistantConversationFilter>(
        'my_assistant_conversation',
        { filter: { conversationId: id } },
      );
      if (res.data?.conversation) {
        setConversationId(res.data.conversation.id);
      }
      setMessages(res.data?.messages ?? []);
    } catch (err) {
      setLoadError(wordedError(err));
      setMessages([]);
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    const initialId = conversationId();
    if (initialId) {
      void loadConversation(initialId);
    }
  });

  let sendSeq = 0;

  const sendMessage = async (overrideText?: string) => {
    const isFromInput = overrideText === undefined;
    const textToSend = (overrideText ?? inputText()).trim();
    if (!textToSend || sending()) return;

    const mine = ++sendSeq;
    if (isFromInput) {
      setInputText('');
    }
    setSending(true);
    setSendError(null);

    try {
      const currentLocale = lang();
      const res = await askWrite.send({
        text: textToSend,
        branch: effectiveBranch(),
        locale: currentLocale,
      });
      if (mine !== sendSeq) return;

      // Update URL to preserve conversation across reloads
      if (typeof window !== 'undefined' && window.history && res.conversationId) {
        try {
          const url = new URL(window.location.href);
          url.searchParams.set('conversation', res.conversationId);
          window.history.replaceState({}, '', url.toString());
        } catch {
          // ignore URL replace failure in test environments
        }
      }

      setConversationId(res.conversationId);
      setMessages((prev) => [...prev, res.customerMessage, res.assistantReply]);
    } catch (err) {
      if (mine !== sendSeq) return;
      if (isFromInput) {
        setInputText(textToSend);
      }
      setSendError(wordedError(err));
    } finally {
      setSending(false);
    }
  };

  const resetConversation = () => {
    sendSeq++;
    setConversationId(null);
    setMessages([]);
    setLoadError(null);
    setSendError(null);
    setInputText('');
    askWrite.reset();
    if (typeof window !== 'undefined' && window.history) {
      try {
        const url = new URL(window.location.href);
        url.searchParams.delete('conversation');
        url.searchParams.delete('conversationId');
        window.history.replaceState({}, '', url.toString());
      } catch {
        // ignore
      }
    }
  };

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    void sendMessage();
  };

  return (
    <div class="assistant-container">
      <header class="assistant-header">
        <div class="assistant-header-top">
          <h1 class="assistant-title">{t('title')}</h1>
          <div class="assistant-actions">
            <Show when={messages().length > 0}>
              <button
                type="button"
                class="assistant-reset-button"
                onClick={resetConversation}
              >
                {t('newConversation')}
              </button>
            </Show>
          </div>
        </div>
        <p class="assistant-subtitle">{t('subtitle')}</p>
        <div class="assistant-disclaimer-banner" role="note">
          {t('disclaimer')}
        </div>
      </header>

      <main class="assistant-body">
        <Show when={loading()}>
          <div role="status" class="assistant-loading">
            {t('loading')}
          </div>
        </Show>

        <Show when={!loading() && loadError() !== null}>
          <div role="alert" class="assistant-error">
            <p>{loadError()}</p>
            <button
              type="button"
              class="assistant-retry-button"
              onClick={() => void loadConversation()}
            >
              {t('retry')}
            </button>
          </div>
        </Show>

        <Show when={!loading() && loadError() === null && messages().length === 0}>
          <div class="assistant-empty">
            <p>{t('empty')}</p>
            <div class="assistant-prompts">
              <button
                type="button"
                class="assistant-prompt-button"
                onClick={() => void sendMessage(t('promptRates'))}
              >
                {t('promptRates')}
              </button>
              <button
                type="button"
                class="assistant-prompt-button"
                onClick={() => void sendMessage(t('promptBranches'))}
              >
                {t('promptBranches')}
              </button>
              <button
                type="button"
                class="assistant-prompt-button"
                onClick={() => void sendMessage(t('promptQueue'))}
              >
                {t('promptQueue')}
              </button>
            </div>
          </div>
        </Show>

        <Show when={!loading() && messages().length > 0}>
          <div class="assistant-messages" role="log" aria-live="polite">
            <For each={messages()}>
              {(msg) => (
                <div
                  class={`message-wrapper ${
                    msg.role === 'CUSTOMER' ? 'message-customer' : 'message-assistant'
                  }`}
                >
                  <span class="message-sender">
                    {msg.role === 'CUSTOMER' ? t('roleCustomer') : t('roleAssistant')}
                  </span>
                  <div class="message-bubble">
                    {renderTextWithMoney(msg.redactedText)}
                  </div>
                  <time class="message-time">{formatTime(msg.createdAt)}</time>
                </div>
              )}
            </For>
          </div>
        </Show>
      </main>

      <footer class="assistant-footer">
        <Show when={sendError() !== null}>
          <div role="alert" class="assistant-send-error">
            {sendError()}
          </div>
        </Show>

        <form onSubmit={handleSubmit} class="assistant-form">
          <input
            type="text"
            class="assistant-input"
            value={inputText()}
            onInput={(e) => setInputText(e.currentTarget.value)}
            placeholder={t('inputPlaceholder')}
            disabled={sending()}
            aria-label={t('inputPlaceholder')}
          />
          <button
            type="submit"
            class="assistant-send-button"
            disabled={sending() || inputText().trim() === ''}
          >
            {sending() ? t('sending') : t('send')}
          </button>
        </form>
        <p class="assistant-security-note">{t('securityNote')}</p>
      </footer>
    </div>
  );
}
