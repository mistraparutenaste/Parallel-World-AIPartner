import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';
import { live2dDevAssets } from './live2d-dev-plugin';

export default defineConfig({
  // Proprietary development assets are served only by an explicit Phase 3
  // dev mount. Vite must never copy public files into normal/Tauri builds.
  publicDir: false,
  plugins: [
    live2dDevAssets(resolve(import.meta.dirname, '../../.dev-assets/live2d')),
    react(),
  ],
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
