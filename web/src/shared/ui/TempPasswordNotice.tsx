import type { JSX } from 'solid-js';

export interface TempPasswordNoticeProps {
  password: string;
  title: string;
  note: string;
  copyLabel: string;
  doneLabel: string;
  onDone: () => void;
}

export function TempPasswordNotice(props: TempPasswordNoticeProps): JSX.Element {
  return (
    <div data-temp-password="">
      <h3>{props.title}</h3>
      <p>{props.note}</p>
      <p><code data-temp-password-value="">{props.password}</code></p>
      <button type="button" data-temp-password-copy="" onClick={() => void navigator.clipboard.writeText(props.password)}>
        {props.copyLabel}
      </button>
      <button type="button" data-temp-password-done="" onClick={() => props.onDone()}>
        {props.doneLabel}
      </button>
    </div>
  );
}
