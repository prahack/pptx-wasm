import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react()],
  // The viewer's WASM glue fetches a sibling .wasm; pre-bundling would relocate the JS
  // and break that relative resolution.
  optimizeDeps: { exclude: ['pptx-viewer'] },
});
