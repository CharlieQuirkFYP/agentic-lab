import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const phemeTarget = process.env.VITE_PHEME_API_URL ?? 'http://127.0.0.1:8000'

export default defineConfig({
  plugins: [react()],
  server: {
    host: true,
    proxy: {
      '/pheme': {
        target: phemeTarget,
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/pheme/, ''),
      },
    },
  },
})
