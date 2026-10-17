import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit(), svelteTesting()],
  server: {
    // Development: proxy the API to a local `abshare serve` (see docs/BUILDING.md).
    proxy: {
      '/v1': process.env.ABSHARE_SERVER ?? 'http://127.0.0.1:8743',
      '/healthz': process.env.ABSHARE_SERVER ?? 'http://127.0.0.1:8743',
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['tests/**/*.test.ts'],
  },
});
