import { createSignal, For, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import type { ProfitSummary, ProfitSummaryRequest } from '../../../../shared/api/generated/profit_summary';
import { errorText, locale, screenT } from '../../../../shared/i18n';
import { MoneyText } from '../../../../shared/ui/MoneyText';
import * as table from './i18n';

const t = screenT('admin.cash.calculators', table);

type FieldKey = keyof ProfitSummaryRequest;
type TextKey = keyof typeof table.th;

const FIELDS: { key: FieldKey; label: TextKey; hint: TextKey; min: boolean }[] = [
  { key: 'realized', label: 'realizedLabel', hint: 'realizedHint', min: false },
  { key: 'foreignValue', label: 'foreignValueLabel', hint: 'foreignValueHint', min: true },
  { key: 'foreignCost', label: 'foreignCostLabel', hint: 'foreignCostHint', min: true },
  { key: 'netAssets', label: 'netAssetsLabel', hint: 'netAssetsHint', min: false },
  { key: 'capital', label: 'capitalLabel', hint: 'capitalHint', min: true },
  { key: 'reserve', label: 'reserveLabel', hint: 'reserveHint', min: true },
  { key: 'cash', label: 'cashLabel', hint: 'cashHint', min: true },
  { key: 'cashMinimum', label: 'cashMinimumLabel', hint: 'cashMinimumHint', min: true },
];

const CARDS: { title: TextKey; key: keyof ProfitSummary; note: TextKey }[] = [
  { title: 'cardRealized', key: 'realized', note: 'cardRealizedNote' },
  { title: 'cardUnrealized', key: 'unrealized', note: 'cardUnrealizedNote' },
  { title: 'cardWithdrawable', key: 'withdrawable', note: 'cardWithdrawableNote' },
];

export function ProfitSummaryPanel(props: { api: ApiClient; branch: string; shiftId: string }): JSX.Element {
  const [values, setValues] = createSignal<Record<string, string>>({});
  const [result, setResult] = createSignal<ProfitSummary | null>(null);
  const [alert, setAlert] = createSignal('');
  let seq = 0;

  const submit = async (e: Event): Promise<void> => {
    e.preventDefault();
    const id = ++seq;
    const body: Record<string, string> = { branch: props.branch };
    for (const f of FIELDS) body[f.key] = (values()[f.key] ?? '').trim();
    try {
      const a = await props.api.act<Record<string, string>, ProfitSummary>('compute', 'profit_summary', body);
      if (id !== seq) return;
      setResult(a);
      setAlert(t('profitComputed', { when: new Date().toLocaleString(locale()) }));
    } catch (err) {
      if (id !== seq) return;
      setResult(null);
      setAlert(err instanceof ApiError ? errorText(err.code, err.message) : errorText('serverError', String(err)));
    }
  };

  return (
    <section class="calc-panel profit-summary">
      <h3>{t('profitHeading')}</h3>
      <p class="dialog-note">{t('profitNote')}</p>
      <div class="profit-cards">
        <For each={CARDS}>
          {(c) => (
            <div class="profit-card">
              <h4>{t(c.title)}</h4>
              <strong>{result() ? <MoneyText value={result()![c.key]} currency="THB" /> : t('waiting')}</strong>
              <small>{t(c.note)}</small>
            </div>
          )}
        </For>
      </div>
      <details>
        <summary>{t('inputsSummary')}</summary>
        <form
          onSubmit={submit}
          onInput={() => {
            seq += 1;
            setResult(null);
            setAlert(t('changed'));
          }}
        >
          <div class="form-grid">
            <For each={FIELDS}>
              {(f) => (
                <label>
                  {t(f.label)}
                  <input
                    type="number"
                    step="0.01"
                    required
                    min={f.min ? '0' : undefined}
                    placeholder={t(f.hint)}
                    value={values()[f.key] ?? ''}
                    onInput={(e) => setValues({ ...values(), [f.key]: e.currentTarget.value })}
                  />
                  <small>{t(f.hint)}</small>
                </label>
              )}
            </For>
          </div>
          <p>{t('profitParagraph')}</p>
          <label>
            <input type="checkbox" required />
            {t('confirm')}
          </label>
          <button class="primary">{t('profitSubmit')}</button>
          <p role="alert">{alert()}</p>
        </form>
      </details>
      <p class="footnote">{t('profitFootnote')}</p>
    </section>
  );
}
