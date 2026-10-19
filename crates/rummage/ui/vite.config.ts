import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The page is served under a base path that is only known at request time (the server adds
// a <base href> to index.html), so every asset URL is relative.
export default defineConfig({
  base: './',
  plugins: [svelte()],
  build: {
    target: 'es2022',
    assetsInlineLimit: 0,
    reportCompressedSize: true,
  },
  server: {
    proxy: { '/api': process.env.RUMMAGE_API ?? 'http://127.0.0.1:8099' },
  },
});
