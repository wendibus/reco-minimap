import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/postcss';
import { defineConfig } from 'vite';

// The interface is a plain client-side app, like the one of Reco Trainer. `npm run build` writes it
// with fixed file names to src/tools/webui/, which the Rust program embeds and serves itself -
// people who use reco-minimap need neither Node.js nor npm.
export default defineConfig({
  plugins: [react()],
  css: { postcss: { plugins: [tailwindcss()] } },
  build: {
    outDir: '../src/tools/webui',
    emptyOutDir: true,
    assetsInlineLimit: 0,
    rollupOptions: { output: { entryFileNames: 'app.js', chunkFileNames: 'app-[name].js', assetFileNames: 'app.[ext]' } },
  },
  // `npm run dev` forwards the API to a running `reco-minimap ui --no-open` on its default port
  server: { host: 'localhost', port: 5178, proxy: { '/api': 'http://127.0.0.1:8788', '/i18n.js': 'http://127.0.0.1:8788', '/docs': 'http://127.0.0.1:8788' } },
});
