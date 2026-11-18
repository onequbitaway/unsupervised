import { defineConfig } from 'vitest/config';

// The viewer is a static page. The server serves `dist/` at `/`.
// In development, `npm run dev` sends the API calls to a running server.
const server = process.env.UNSUP_SERVER ?? 'http://127.0.0.1:8080';

export default defineConfig({
  base: './',
  build: {
    outDir: 'dist',
    target: 'es2022',
    // PixiJS is one large module. The warning limit is above its size.
    chunkSizeWarningLimit: 900,
  },
  server: {
    proxy: {
      '/v1': { target: server, ws: true },
    },
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
});
