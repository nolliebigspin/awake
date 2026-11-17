// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";

export default defineConfig({
  site: "https://awake-site.vercel.app",
  trailingSlash: "never",
  build: {
    // privacy.html + Vercel cleanUrls => /privacy, matching the old URLs.
    format: "file",
    // The CSP allows only same-origin files (no 'unsafe-inline'), so never
    // inline CSS or JS into the HTML.
    inlineStylesheets: "never",
  },
  vite: { build: { assetsInlineLimit: 0 } },
  integrations: [sitemap()],
});
