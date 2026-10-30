// Pricing formula FE T7 (docs/design/blueprint/pricing-formula-web.md §1, §3 V1–V8): the branch override block at the
// foot of the policy card. Collapsed; reads `pricing_formula` only when opened (RatesDraft.test.tsx:607 pins the
// reads on show).
import { createEffect, createSignal, on, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { FormulaTextsDto, PricingFormulaData } from '../../../shared/api/generated/pricing_formula';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { FormulaEditor } from '../parts/FormulaEditor';
import * as formulaI18n from '../parts/formula-i18n';
import { createFormulaPreview } from '../parts/formula-preview';
import { FormulaPreviewTable } from '../parts/FormulaPreviewTable';
import { EMPTY_OVERRIDE, initialOverride, type FormulaOverrideState } from './formula-override';
import '../parts/formula.css';

const t = screenT('admin.formula', formulaI18n);

const EMPTY_TEXTS: FormulaTextsDto = { buyFormula: '', sellFormula: '' };

function sourceText(source: string): string {
  if (source === 'BRANCH') return t('source.BRANCH');
  if (source === 'ORGANISATION') return t('source.ORGANISATION');
  return t('source.DEFAULT');
}

export interface RatesFormulaOverrideProps {
  branch: () => string;
  state: () => FormulaOverrideState;
  setState: (s: FormulaOverrideState) => void;
  /** Moves after a successful policy save: an open block reads the view again. */
  reloadKey: () => number;
  disabled: () => boolean;
}

export function RatesFormulaOverride(props: RatesFormulaOverrideProps): JSX.Element {
  const api = new ApiClient();
  const [open, setOpen] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [orgTexts, setOrgTexts] = createSignal<FormulaTextsDto>(EMPTY_TEXTS);
  const [defaults, setDefaults] = createSignal<FormulaTextsDto>(EMPTY_TEXTS);
  const [inputs, setInputs] = createSignal<readonly string[]>([]);
  let reqGen = 0;

  const preview = createFormulaPreview({
    api,
    branch: props.branch,
    scope: 'BRANCH',
    buy: () => props.state().buy,
    sell: () => props.state().sell,
    enabled: () => open() && props.state().current !== null && props.state().choice === 'BRANCH',
  });

  async function load(branch: string): Promise<void> {
    const myGen = ++reqGen;
    setError(null);
    try {
      const ans = await api.view<PricingFormulaData>('pricing_formula', { branch });
      if (myGen !== reqGen || !open() || props.branch() !== branch) return;
      const data = ans.data;
      if (!data) throw new Error('missing pricing_formula data');
      const org = data.organisation ?? data.default;
      const entry = data.branches.find((b) => b.branch === branch) ?? {
        branch,
        source: data.organisation ? 'ORGANISATION' : 'DEFAULT',
        own: null,
      };
      setOrgTexts({ buyFormula: org.buyFormula, sellFormula: org.sellFormula });
      setDefaults(data.default);
      setInputs(data.inputs.map((i) => i.name));
      props.setState(initialOverride(entry, org));
    } catch (err: unknown) {
      if (myGen !== reqGen || !open() || props.branch() !== branch) return;
      if (props.state().current !== null) {
        props.setState(EMPTY_OVERRIDE);
      }
      setError(wordedError(err));
    }
  }

  function handleOpen(): void {
    setOpen(true);
    void load(props.branch());
  }

  createEffect(
    on(
      props.reloadKey,
      () => {
        if (open()) {
          void load(props.branch());
        }
      },
      { defer: true },
    ),
  );

  createEffect(
    on(
      props.branch,
      () => {
        reqGen++;
        setOpen(false);
        setError(null);
        props.setState(EMPTY_OVERRIDE);
      },
      { defer: true },
    ),
  );

  return (
    <div class="formula-override">
      <Show when={!open() || props.state().current === null}>
        <button
          type="button"
          data-action="formula-override-open"
          disabled={props.disabled()}
          onClick={handleOpen}
        >
          {t('overrideOpen')}
        </button>
      </Show>
      <Show when={open()}>
        <Show when={error()}>
          {(msg) => (
            <div class="form-error" role="alert" data-testid="formula-override-error">
              {msg()}
            </div>
          )}
        </Show>
        <Show when={props.state().current}>
          {(cur) => (
            <>
              <h3>{t('overrideTitle')}</h3>
              <div class="formula-source" data-testid="formula-override-source">
                {sourceText(cur().source)}
              </div>
              <div class="formula-override-choice">
                <label>
                  <input
                    type="radio"
                    name="formula-choice"
                    value="ORGANISATION"
                    checked={props.state().choice === 'ORGANISATION'}
                    disabled={props.disabled()}
                    onChange={() => props.setState({ ...props.state(), choice: 'ORGANISATION' })}
                  />
                  <span>{t('overrideFollow')}</span>
                </label>
                <label>
                  <input
                    type="radio"
                    name="formula-choice"
                    value="BRANCH"
                    checked={props.state().choice === 'BRANCH'}
                    disabled={props.disabled()}
                    onChange={() => props.setState({ ...props.state(), choice: 'BRANCH' })}
                  />
                  <span>{t('overrideOwn')}</span>
                </label>
              </div>
              <Show when={props.state().choice === 'ORGANISATION'}>
                <div class="formula-override-org" data-testid="formula-override-org">
                  <div>{t('overrideOrgText')}</div>
                  <div>
                    <span>{t('buyFormulaLabel')}: </span>
                    <code>{orgTexts().buyFormula}</code>
                  </div>
                  <div>
                    <span>{t('sellFormulaLabel')}: </span>
                    <code>{orgTexts().sellFormula}</code>
                  </div>
                </div>
              </Show>
              <Show when={props.state().choice === 'BRANCH'}>
                <FormulaEditor
                  buy={() => props.state().buy}
                  sell={() => props.state().sell}
                  onBuy={(buy) => props.setState({ ...props.state(), choice: 'BRANCH', buy })}
                  onSell={(sell) => props.setState({ ...props.state(), choice: 'BRANCH', sell })}
                  inputs={inputs}
                  defaults={defaults}
                  parseError={() => preview.result()?.parseError ?? null}
                  disabled={props.disabled}
                />
                <div data-testid="formula-override-note">{t('overridePreviewNote')}</div>
                <FormulaPreviewTable
                  result={preview.result}
                  pending={preview.pending}
                  error={preview.error}
                />
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
