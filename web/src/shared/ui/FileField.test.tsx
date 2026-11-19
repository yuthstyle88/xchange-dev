// OL-7 rows 12 + 101 (VERIFY.md on main 4a577dac, shots 007 and 056): /desk/backdated and /operations/verification
// still show the browser's own "Choose File / No file chosen" (English, truncated to "No …sen" at 1280); xc-theme.css
// only restyles ::file-selector-button, and the browser text beside it cannot be translated or styled. The root fix is
// ONE shared part, FileField: the native input stays (same name, accept, change event, ref, so screens keep their
// handlers and tests), hidden from sight but focusable, followed by a theme secondary-button "เลือกไฟล์" and the picked
// file's name or "ยังไม่ได้เลือกไฟล์". It sits inside the screen's <label>, so a click anywhere on it opens the picker.
// Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import * as fileFieldModule from './FileField';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Props = {
  name: string;
  accept: string;
  file: File | null | undefined;
  onChange: (e: Event & { currentTarget: HTMLInputElement }) => void;
  ref?: (el: HTMLInputElement) => void;
  disabled?: boolean;
};

let readHere: (name: string) => string = () => '';

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim().replace(/\s+/g, ' ')).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

function mount(props: () => Props): void {
  const FileField = (fileFieldModule as unknown as { FileField?: (p: Props) => unknown }).FileField;
  expect(typeof FileField, 'shared/ui/FileField.tsx exports FileField').toBe('function');
  dispose = render(
    () => (
      <label>
        <span>สลิป</span>
        {FileField!(props()) as never}
      </label>
    ),
    host,
  );
}

const text = (sel: string) => host.querySelector(sel)?.textContent?.trim();

describe('FileField', () => {
  it('FF-1 keeps the native input (name, accept) inside .xc-file-field, then a pick button and the empty name', () => {
    mount(() => ({ name: 'slip', accept: 'image/jpeg,application/pdf', file: undefined, onChange: () => {} }));
    const root = host.querySelector('label > .xc-file-field');
    expect(root, 'label > .xc-file-field').not.toBeNull();
    const input = root!.querySelector<HTMLInputElement>('input.xc-file-field-input[type="file"]');
    expect(input).not.toBeNull();
    expect(input!.name).toBe('slip');
    expect(input!.accept).toBe('image/jpeg,application/pdf');
    expect(Array.from(root!.children).map((c) => c.className)).toEqual([
      'xc-file-field-input',
      'xc-file-field-pick',
      'xc-file-field-name',
    ]);
    expect(text('.xc-file-field-pick')).toBe('เลือกไฟล์');
    expect(root!.querySelector('.xc-file-field-pick')!.getAttribute('aria-hidden')).toBe('true');
    expect(text('.xc-file-field-name')).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it('FF-2 shows the picked file name, and the empty text again when the screen clears it', () => {
    const [file, setFile] = createSignal<File | null>(new File(['x'], 'slip-0711.jpg', { type: 'image/jpeg' }));
    mount(() => ({ name: 'slip', accept: 'image/jpeg', get file() { return file(); }, onChange: () => {} }) as Props);
    expect(text('.xc-file-field-name')).toBe('slip-0711.jpg');
    setFile(null);
    expect(text('.xc-file-field-name')).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it('FF-3 passes the native change event and the input element through', () => {
    let seen: HTMLInputElement | undefined;
    let changed: EventTarget | null = null;
    mount(() => ({
      name: 'file',
      accept: 'image/png',
      file: null,
      ref: (el) => (seen = el),
      onChange: (e) => (changed = e.currentTarget),
    }));
    const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
    expect(seen).toBe(input);
    input.dispatchEvent(new Event('change', { bubbles: true }));
    expect(changed).toBe(input);
  });

  it('FF-4 disabled reaches the input', () => {
    mount(() => ({ name: 'file', accept: 'image/png', file: null, onChange: () => {}, disabled: true }));
    expect(host.querySelector<HTMLInputElement>('input[type="file"]')!.disabled).toBe(true);
  });

  it('FF-5 the texts follow the language: th, en, zh', () => {
    expect(th['field.file.choose' as keyof typeof th]).toBe('เลือกไฟล์');
    expect(th['field.file.none' as keyof typeof th]).toBe('ยังไม่ได้เลือกไฟล์');
    expect(en['field.file.choose' as keyof typeof en]).toBe('Choose file');
    expect(en['field.file.none' as keyof typeof en]).toBe('No file chosen');
    expect(zh['field.file.choose' as keyof typeof zh]).toBe('选择文件');
    expect(zh['field.file.none' as keyof typeof zh]).toBe('未选择文件');
    setLang('en');
    mount(() => ({ name: 'file', accept: 'image/png', file: null, onChange: () => {} }));
    expect(text('.xc-file-field-pick')).toBe('Choose file');
    expect(text('.xc-file-field-name')).toBe('No file chosen');
  });

  it('FF-6 file-field.css: the native input is hidden but focusable; the pick is a theme secondary button', () => {
    expect(readHere('FileField.tsx')).toMatch(/import '\.\/file-field\.css';/);
    const css = readHere('file-field.css');
    const root = decls(css, '.xc-file-field');
    expect(root?.get('display')).toBe('flex');
    expect(root?.get('align-items')).toBe('center');
    expect(root?.get('gap')).toBe('12px');
    expect(root?.get('position')).toBe('relative');
    const input = decls(css, '.xc-file-field .xc-file-field-input');
    expect(input?.get('position')).toBe('absolute');
    expect(input?.get('width')).toBe('1px');
    expect(input?.get('height')).toBe('1px');
    expect(input?.get('opacity')).toBe('0');
    expect(input?.get('overflow')).toBe('hidden');
    expect(input?.get('clip-path')).toBe('inset(50%)');
    expect(input?.get('display')).toBeUndefined();
    const pick = decls(css, '.xc-file-field-pick');
    expect(pick?.get('border')).toBe('1px solid var(--xc-line)');
    expect(pick?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(pick?.get('background')).toBe('var(--xc-surface)');
    expect(pick?.get('color')).toBe('var(--xc-brand-text)');
    expect(pick?.get('min-height')).toBe('40px');
    expect(pick?.get('cursor')).toBe('pointer');
    expect(pick?.get('flex')).toBe('none');
    const name = decls(css, '.xc-file-field-name');
    expect(name?.get('color')).toBe('var(--xc-muted)');
    expect(name?.get('min-width')).toBe('0');
    expect(name?.get('overflow')).toBe('hidden');
    expect(name?.get('text-overflow')).toBe('ellipsis');
    expect(name?.get('white-space')).toBe('nowrap');
  });

  it('FF-7 keyboard focus on the hidden input shows on the pick button; disabled greys it', () => {
    const css = readHere('file-field.css');
    const focus = decls(css, '.xc-file-field-input:focus-visible + .xc-file-field-pick');
    expect(focus?.get('outline')).toBe('3px solid var(--xc-brand)');
    expect(focus?.get('outline-offset')).toBe('2px');
    const off = decls(css, '.xc-file-field-input:disabled + .xc-file-field-pick');
    expect(off?.get('background')).toBe('var(--xc-gray-100)');
    expect(off?.get('color')).toBe('var(--xc-gray-600)');
    expect(off?.get('cursor')).toBe('not-allowed');
  });
});
