import { defineConfig } from 'vite'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// The Rust binary serves the built SPA and the /api endpoints.
// TOKEN_PROXY_ADMIN_PORT=18788 points the dev server at `cargo run --example demo`.
const adminOrigin = `http://127.0.0.1:${process.env.TOKEN_PROXY_ADMIN_PORT ?? '8788'}`

const config = defineConfig({
  resolve: { tsconfigPaths: true },
  plugins: [tailwindcss(), tanstackStart({ spa: { enabled: true } }), viteReact()],
  server: {
    proxy: {
      // Dev only: forward API calls to the running token-proxy, as if the
      // page were served by it (the admin API checks Host and Origin).
      '/api': {
        target: adminOrigin,
        changeOrigin: true,
        headers: { origin: adminOrigin },
      },
    },
  },
})

export default config
