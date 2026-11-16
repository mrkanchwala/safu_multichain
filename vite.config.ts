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
  const labels = (cfg.chains.labels ?? {}) as Record<string, unknown>
  const unlabelled = enabled.filter((c) => typeof labels[c] !== 'string')
  if (unlabelled.length) throw new Error(`hq/site.toml: chains.labels is missing ${unlabelled.join(', ')}`)
  // [guards] is for the build checks only (scripts/check-dist.mjs): it must never ship in the page.
  const { guards: _guards, ...forPage } = cfg
  return forPage
}

// The Solana pool (B42): config/solana.<cluster>.json (hq/site.toml [chains.solana]) + the deployed program's pinned IDL (config/solana.idl.json,
// the same two files its claim API reads) are baked in only when hq/site.toml
// enables Solana. __CHAIN_SOLANA__ is a build constant, so with Solana off the registry's import is dead
// code and none of the Solana pool's code or addresses reach the bundle (review A3).
function solanaDefines(chains: { enabled: string[]; solana?: { cluster?: string } }) {
  const on = chains.enabled.includes('solana')
  const cluster = chains.solana?.cluster
  if (on && !cluster) throw new Error('hq/site.toml: [chains.solana] cluster is missing')
  const read = (rel: string) => JSON.parse(readFileSync(new URL(rel, import.meta.url), 'utf8'))
  return {
    __CHAIN_SOLANA__: JSON.stringify(on),
    __SOL_POOL__: on ? JSON.stringify(read(`./config/solana.${cluster}.json`)) : 'null',
    __SOL_IDL__: on ? JSON.stringify(read('./config/solana.idl.json')) : 'null',
  }
}

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const site = siteConfig()
  return {
  // Served from the safustaking.com domain root (since 2026-09-25;
  // the older T3 site lives at /t3). Asset URLs are
  // root-absolute (/assets/...), matching nginx's root for this dist.
  base: '/',
  // HOT Wallet's SDK expects Node's global/Buffer -- see src/lib/client.tsx.
  plugins: [react(), nodePolyfills({ include: ['buffer'], globals: { Buffer: true, global: true } })],
  define: {
    'process.env.NODE_ENV': JSON.stringify(mode),
    __POOL__: JSON.stringify(poolConfig(mode)),
    __SITE__: JSON.stringify(site),
    ...solanaDefines(site.chains as { enabled: string[]; solana?: { cluster?: string } }),
  },
  server: {
    // backend/app.py (demo claim API), run separately with uvicorn. The Solana pool's API
    // (colosseum_app.py) sits under /api/solana, as on the live site; SAFU_SOL_API points at it.
    proxy: {
      ...Object.fromEntries(['/claim', '/covered-wallets', '/relay'].map((p) => [p, 'http://localhost:8000'])),
      '/api/solana': {
        target: process.env.SAFU_SOL_API ?? 'http://localhost:8002',
        rewrite: (p: string) => p.replace(/^\/api\/solana/, ''),
      },
    },
  },
  }
})
