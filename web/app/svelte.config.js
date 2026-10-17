import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  preprocess: vitePreprocess(),
  kit: {
    // One static page served by the ABShare server; all data comes from the /v1 API.
    adapter: adapter({ fallback: 'index.html', strict: true }),
    // Hash routes need no server-side fallback, so the server serves plain files only.
    router: { type: 'hash' },
    // Relative asset paths: the app works under any base path.
    paths: { relative: true },
    output: { bundleStrategy: 'split' },
    // Registered by the app (src/lib/pwa/update.svelte.ts), which never re-registers offline.
    serviceWorker: { register: false },
  },
};
