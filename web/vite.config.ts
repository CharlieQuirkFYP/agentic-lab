import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      "/api/v1": "http://127.0.0.1:8080",
    },
  },
  resolve: {
    alias: {
      "@": `${import.meta.dirname}/src`,
    },
  },
});
