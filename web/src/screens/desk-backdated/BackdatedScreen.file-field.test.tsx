// OL-7 row 12 (VERIFY.md on main 4a577dac, shot 007): /desk/backdated still shows two browser "Choose File" inputs
// ("No …sen" truncated at 1280). Both the slip and the passport use the shared FileField part (shared/ui/FileField.tsx,
// ticket file-field), which shows "เลือกไฟล์" and the picked file's name. The screen's own `.desk-file` line keeps only
// its remove button, so the name is shown once. Same name, accept and change handler as before.
// Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { BackdatedScreen } from './BackdatedScreen';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  const api = new ApiClient(
    async () =>
      new Response(JSON.stringify({ version: 1, changed: true, data: { currencies: [], rates: [] } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
  );
  dispose = render(() => <BackdatedScreen api={api} me={() => undefined} />, host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
});

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function choose(name: string, file: File): HTMLInputElement {
  const input = host.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
  Object.defineProperty(input, 'files', { get: () => [file], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  return input;
}

const part = (name: string) => host.querySelector(`input[name="${name}"]`)?.closest('.xc-file-field');
const shown = (name: string) => part(name)?.querySelector('.xc-file-field-name')?.textContent?.trim();

describe('backdated file inputs use the shared FileField', () => {
  it.each(['slip', 'passport'])('BF-1 %s: the input sits in the shared part inside its field label', (name) => {
    const input = host.querySelector<HTMLInputElement>(`input[name="${name}"]`);
    expect(input).not.toBeNull();
    expect(input!.type).toBe('file');
    expect(input!.accept).toBe('.jpg,.jpeg,.png,.webp,.pdf,image/jpeg,image/png,image/webp,application/pdf');
    expect(input!.closest('label.desk-field > .xc-file-field')).not.toBeNull();
    expect(part(name)?.querySelector('.xc-file-field-pick')?.textContent?.trim()).toBe('เลือกไฟล์');
    expect(shown(name)).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it.each(['slip', 'passport'])('BF-2 %s: a picked file is named once, and remove brings back the empty text', async (name) => {
    choose(name, new File(['abc'], `${name}-0711.jpg`, { type: 'image/jpeg' }));
    await flush();
    expect(shown(name)).toBe(`${name}-0711.jpg`);
    const label = host.querySelector(`input[name="${name}"]`)!.closest('label.desk-field')!;
    expect(label.textContent!.split(`${name}-0711.jpg`).length - 1, 'file name shown once').toBe(1);
    const remove = label.querySelector<HTMLButtonElement>('.desk-file button');
    expect(remove, 'the remove button stays').not.toBeNull();
    remove!.click();
    await flush();
    expect(shown(name)).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it('BF-3 the screen writes no <input type="file"> of its own', async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as { readFileSync(f: string, e: 'utf8'): string };
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const src = fs.readFileSync(here + 'BackdatedScreen.tsx', 'utf8');
    expect(src).not.toMatch(/type="file"/);
    expect(src).toMatch(/import \{ FileField \} from '\.\.\/\.\.\/shared\/ui\/FileField';/);
  });
});
