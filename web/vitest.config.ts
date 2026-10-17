import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid()],
  test: {
    environment: 'jsdom',
    // The zone is pinned so a forgotten `timeZone: 'Asia/Bangkok'` goes red on a Bangkok machine too.
    env: { TZ: 'UTC' },
  },
  resolve: {
    conditions: ['development', 'browser'],
  },
});
