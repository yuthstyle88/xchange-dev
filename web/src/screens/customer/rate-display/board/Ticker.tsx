// P01 v2 welcome ticker (rate-display.md §9.2; rulings on PR 3558: the greetings are kept, 中文 is here).
import type { JSX } from 'solid-js';
import { BOARD_TEXT } from './board-texts';
import './ticker.css';

export interface TickerProps {
  count: number;
  attract: boolean;
}

export function Ticker(props: TickerProps): JSX.Element {
  const parts = () => {
    const list: string[] = [
      ...BOARD_TEXT.tickerGreetings,
      BOARD_TEXT.tickerExchangeHere,
    ];
    if (props.count > 0) {
      list.push(
        `${props.count} ${BOARD_TEXT.tickerCount.th} · ${props.count} ${BOARD_TEXT.tickerCount.en} · ${props.count} ${BOARD_TEXT.tickerCount.zh}`,
      );
    }
    list.push(BOARD_TEXT.tickerBetterRates);
    return list;
  };

  const renderLine = () => (
    <span class="xc-rb-ticker-item">
      {parts().map((text, i) => (
        <>
          {i > 0 && <i class="xc-rb-ticker-sep">✦</i>}
          {text}
        </>
      ))}
    </span>
  );

  return (
    <div class="xc-rb-ticker" aria-hidden="true">
      <div
        class="xc-rb-ticker-track"
        classList={{ 'xc-rb-ticker-paused': !props.attract }}
      >
        {renderLine()}
        {renderLine()}
      </div>
    </div>
  );
}
