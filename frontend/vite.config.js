import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 5173,
    open: true,
    proxy: {
      "/challenge": "http://localhost:3000",
      "/verify": "http://localhost:3000",
      "/content": "http://localhost:3000",
    },
  },
});