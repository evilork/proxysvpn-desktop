import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

/**
 * Certificate for testing on a phone, if `scripts/dev-cert.sh` has made one.
 *
 * Safari hands out the motion sensors only in a secure context, and a plain
 * LAN address is not one: over http the living eye cannot work there at all.
 *
 * Behind an explicit switch (`npm run dev:phone`) and not merely behind the
 * files existing, because `tauri dev` expects plain http on localhost and a
 * certificate left lying around would otherwise break the desktop run.
 */
function devHttps(): { key: Buffer; cert: Buffer } | undefined {
  if (!process.env.PROXYS_DEV_HTTPS) return undefined;
  const dir = resolve(__dirname, ".certs");
  const key = resolve(dir, "key.pem");
  const cert = resolve(dir, "cert.pem");
  if (!existsSync(key) || !existsSync(cert)) {
    throw new Error("PROXYS_DEV_HTTPS is set but .certs is empty — run: bash scripts/dev-cert.sh");
  }
  return { key: readFileSync(key), cert: readFileSync(cert) };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    https: devHttps(),
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
