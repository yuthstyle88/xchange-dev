import type { JSX } from 'solid-js';
import { t } from '../i18n';
import './file-field.css';

export function FileField(props: {
  name: string;
  accept: string;
  file: File | null | undefined;
  onChange: (e: Event & { currentTarget: HTMLInputElement }) => void;
  ref?: (el: HTMLInputElement) => void;
  disabled?: boolean;
}): JSX.Element {
  return (
    <span class="xc-file-field">
      <input
        class="xc-file-field-input"
        type="file"
        name={props.name}
        accept={props.accept}
        disabled={props.disabled}
        ref={(el) => props.ref?.(el)}
        onChange={(e) => props.onChange(e)}
      />
      <span class="xc-file-field-pick" aria-hidden="true">
        {t('field.file.choose')}
      </span>
      <span class="xc-file-field-name">{props.file?.name ?? t('field.file.none')}</span>
    </span>
  );
}
