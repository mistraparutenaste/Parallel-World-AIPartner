import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Three independent window entries; each webview loads exactly one.
export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      input: {
        character: resolve(import.meta.dirname, 'character.html'),
        chat: resolve(import.meta.dirname, 'chat.html'),
        settings: resolve(import.meta.dirname, 'settings.html'),
      },
    },
  },
});
