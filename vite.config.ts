import { defineConfig } from 'vite';

// Porta fixa: o Tauri aponta o devUrl pra cá (tauri.conf.json > build.devUrl)
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: 'es2021',
    minify: 'esbuild',
  },
});
