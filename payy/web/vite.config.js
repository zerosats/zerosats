import { defineConfig } from 'vite';
import { resolve } from 'path';

export default defineConfig({
  // Base path - empty for Electron file:// protocol
  base: './',
  
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
