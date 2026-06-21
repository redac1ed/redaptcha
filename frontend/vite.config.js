import { defineConfig } from "vite";

export default defineConfig(({ mode }) => {
  if (mode === "embed") {
    return {
      build: {
        lib: {
          entry: "src/embed.js",
          formats: ["es"],
          fileName: () => "redaptcha.js",
        },
        outDir: "dist-embed",
        emptyOutDir: true,
        cssCodeSplit: false,
      },
    };
  }
  return {
    server: {
      port: 5173,
      open: true,
      proxy: {
        "^/challenge(/.*)?$": "http://localhost:3000",
        "/verify": "http://localhost:3000",
        "/content": "http://localhost:3000",
        "/siteverify": "http://localhost:3000",
      },
    },
  };
});