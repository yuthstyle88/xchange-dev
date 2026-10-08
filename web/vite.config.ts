import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function getInputs(): Record<string, string> {
  const pagesDir = path.resolve(__dirname, 'pages');
  const inputs: Record<string, string> = {};
  if (fs.existsSync(pagesDir)) {
    const files = fs.readdirSync(pagesDir);
    for (const file of files) {
      if (file.endsWith('.html')) {
        const name = path.basename(file, '.html');
        inputs[name] = path.resolve(pagesDir, file);
      }
    }
  }
  if (Object.keys(inputs).length === 0) {
    inputs['client'] = path.resolve(__dirname, 'src/shared/api/client.ts');
  }
  return inputs;
}

export default defineConfig({
  base: '/ui/',
  plugins: [solid()],
  build: {
    outDir: 'dist',
    manifest: true,
    emptyOutDir: true,
    rollupOptions: {
      input: getInputs(),
    },
  },
});
