import { readFileSync } from 'node:fs'
import react from '@vitejs/plugin-react'
import { parse as parseToml } from 'smol-toml'
import { defineConfig } from 'vite'
import { nodePolyfills } from 'vite-plugin-node-polyfills'

// The pool's ONE network switch (C1, 2026-09-29): SAFU_POOL_NETWORK=testnet|mainnet picks
// config/pool.<network>.json, the same file the backend reads (backend/pool_net.py). No default for a
// build; a blank value on the chosen network fails the build (the backend refuses to start the same way).
function poolConfig(mode: string) {
  const net = process.env.SAFU_POOL_NETWORK ?? (mode === 'test' ? 'testnet' : '')
  if (net !== 'testnet' && net !== 'mainnet') throw new Error(`SAFU_POOL_NETWORK=${JSON.stringify(net)}; set testnet or mainnet`)
  const cfg = JSON.parse(readFileSync(new URL(`./config/pool.${net}.json`, import.meta.url), 'utf8'))
  const blank: string[] = []
  const walk = (node: unknown, path: string) => {
    if (node === null) blank.push(path)
    else if (typeof node === 'object') for (const [k, v] of Object.entries(node as object)) walk(v, `${path}.${k}`)
  }
  walk(cfg, net)
  if (blank.length) throw new Error(`pool.${net}.json has blank values: ${blank.join(', ')} (deploy first)`)
  return cfg
}

// Site HQ (B42): hq/site.toml, the site's own values. Every key the site reads must be present,
// and the default chain must be an enabled one; anything else fails the build.
const SITE_KEYS = {
  site: ['name', 'url', 'icon', 'wallet_description', 'walletconnect_project_id'],
  links: ['feedback', 'x', 'telegram', 'github', 'bd_intake'],
  chains: ['enabled', 'default'],
} as const
// Which chains have an adapter is checked by src/chains/registry.test.ts (one list: the registry).

function siteConfig() {
  const cfg = parseToml(readFileSync(new URL('./hq/site.toml', import.meta.url), 'utf8')) as Record<string, Record<string, unknown>>
  const missing = Object.entries(SITE_KEYS).flatMap(([t, keys]) => keys.filter((k) => cfg[t]?.[k] == null).map((k) => `${t}.${k}`))
  if (missing.length) throw new Error(`hq/site.toml is missing: ${missing.join(', ')}`)
  const enabled = cfg.chains.enabled as string[]
  if (!enabled.includes(cfg.chains.default as string)) throw new Error('hq/site.toml: chains.default is not enabled')
  return cfg
}

// https://vite.dev/config/
export default defineConfig(({ mode }) => ({
  // Served from the safustaking.com domain root (since 2026-09-25;
  // the older T3 site lives at /t3). Asset URLs are
  // root-absolute (/assets/...), matching nginx's root for this dist.
  base: '/',
  // HOT Wallet's SDK expects Node's global/Buffer -- see src/lib/client.tsx.
  plugins: [react(), nodePolyfills({ include: ['buffer'], globals: { Buffer: true, global: true } })],
  define: {
    'process.env.NODE_ENV': JSON.stringify(mode),
    __POOL__: JSON.stringify(poolConfig(mode)),
    __SITE__: JSON.stringify(siteConfig()),
  },
  server: {
    // backend/app.py (demo claim API), run separately with uvicorn.
    proxy: Object.fromEntries(['/claim', '/covered-wallets', '/relay'].map((p) => [p, 'http://localhost:8000'])),
  },
}))
