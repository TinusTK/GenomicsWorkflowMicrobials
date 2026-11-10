import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

// `npm run dev` talks to a `gwm serve` on port 8642
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  base: './',
  server: { port: 5173, proxy: { '/api': 'http://127.0.0.1:8642' } },
  build: { outDir: 'dist', emptyOutDir: true },
});
