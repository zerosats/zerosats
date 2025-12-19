import { defineConfig } from 'vite';
import { resolve } from 'path';
import { readFileSync } from 'fs';

const pkg = JSON.parse(readFileSync('./package.json', 'utf-8'));

export default defineConfig({
  // Base path - empty for Electron file:// protocol
  base: './',

  // Constants
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version)
  },

  // Resolve aliases
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },
  
  // Build settings
  build: {
    target: 'esnext',
    outDir: 'dist',
    emptyOutDir: true,
    // Generate source maps for debugging
    sourcemap: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
      },
    },
  },
  
  // Development server
  server: {
    port: 3000,
    strictPort: true,
  },
});
