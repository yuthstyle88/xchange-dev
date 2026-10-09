import { beforeAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { CheckField } from '../../../shared/ui/Field';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}

interface NodePath {
  resolve(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let adminCss: string;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisFile = new URL(import.meta.url).pathname;
  const thisDir = path.dirname(thisFile);
  const adminCssPath = path.resolve(thisDir, '../admin.css');
  adminCss = fs.readFileSync(adminCssPath, 'utf8');
});

describe('CheckField in admin look (#714 CD-2)', () => {
  it('renders CheckField with input checkbox followed by check-text label', () => {
    const container = document.createElement('div');
    container.className = 'xc-admin';
    const content = document.createElement('div');
    content.className = 'xc-content';
    container.appendChild(content);
    document.body.appendChild(container);

    let checkedVal = false;
    const dispose = render(
      () => (
        <CheckField
          label="เปิดใช้"
          hint="รับเงินบาทจากลูกค้า"
          checked={checkedVal}
          onChange={(c) => {
            checkedVal = c;
          }}
        />
      ),
      content
    );

    const labelEl = content.querySelector('label.check-field') as HTMLElement;
    expect(labelEl).toBeTruthy();

    const inputEl = labelEl.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(inputEl).toBeTruthy();
    expect(inputEl.type).toBe('checkbox');

    const checkTextEl = labelEl.querySelector('.check-text') as HTMLElement;
    expect(checkTextEl).toBeTruthy();
    expect(checkTextEl.textContent).toContain('เปิดใช้');
    expect(checkTextEl.textContent).toContain('รับเงินบาทจากลูกค้า');

    // Box sits beside text: input is a flex sibling of check-text within label.check-field
    expect(labelEl.children[0]).toBe(inputEl);
    expect(labelEl.children[1]).toBe(checkTextEl);

    dispose();
    container.remove();
  });

  it('asserts every admin.css selector whose last compound is label, label input or label select carries :not(.check-field)', () => {
    // Rule :77 label must carry :not(.check-field) guard
    expect(adminCss).toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label:not\(\.check-field\)/);
    // Bare .xc-admin .xc-content.xc-content label { without :not(.check-field) must not exist
    expect(adminCss).not.toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label\s*\{/);

    // Rule :83 label input, label select must carry :not(.check-field) guard
    expect(adminCss).toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label:not\(\.check-field\)\s+input/);
    expect(adminCss).toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label:not\(\.check-field\)\s+select/);
    expect(adminCss).not.toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label\s+input/);
    expect(adminCss).not.toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+label\s+select/);

    // Rule :63 input must not apply 100% width/min-height/padding to checkboxes
    expect(adminCss).toMatch(/\.xc-admin\s+\.xc-content\.xc-content\s+input:not\(\[type="checkbox"\]\)/);
  });
});
