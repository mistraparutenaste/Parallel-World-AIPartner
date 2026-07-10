import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react()],
  build: {
    rolldownOptions: {
      input: {
        character: resolve(import.meta.dirname, 'character.html'),
        chat: resolve(import.meta.dirname, 'chat.html'),
        settings: resolve(import.meta.dirname, 'settings.html'),
      },
    },
  },
});
