import { readFileSync } from 'node:fs'
import react from '@vitejs/plugin-react'
import { parse as parseToml } from 'smol-toml'
import { defineConfig } from 'vite'
import { configDefaults } from 'vitest/config'
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
  site: ['name', 'url', 'icon', 'walletconnect_project_id'],
  hero: ['headline', 'accent', 'tagline'],
  links: ['feedback', 'x', 'telegram', 'github', 'bd_intake'],
  chains: ['enabled', 'default'],
} as const
// Which chains have an adapter is checked by src/chains/registry.test.ts (one list: the registry).
const PARTNER_KEYS = ['row', 'name', 'url', 'logo', 'logo_text', 'show'] as const

function siteConfig() {
  const cfg = parseToml(readFileSync(new URL('./hq/site.toml', import.meta.url), 'utf8')) as Record<string, Record<string, unknown>>
  const missing = Object.entries(SITE_KEYS).flatMap(([t, keys]) => keys.filter((k) => cfg[t]?.[k] == null).map((k) => `${t}.${k}`))
  if (missing.length) throw new Error(`hq/site.toml is missing: ${missing.join(', ')}`)
  const enabled = cfg.chains.enabled as string[]
  if (!enabled.includes(cfg.chains.default as string)) throw new Error('hq/site.toml: chains.default is not enabled')
  const labels = (cfg.chains.labels ?? {}) as Record<string, unknown>
  const unlabelled = enabled.filter((c) => typeof labels[c] !== 'string')
  if (unlabelled.length) throw new Error(`hq/site.toml: chains.labels is missing ${unlabelled.join(', ')}`)
  // [[partners]] (M18): every entry needs these keys; only shown ones ship, so a partner still waiting
  // on an OK never appears in the page or its bundle.
  const partners = (cfg.partners ?? []) as unknown as Record<string, unknown>[]
  const badPartners = partners.filter((p) => PARTNER_KEYS.some((k) => p[k] == null)).map((p) => String(p.name ?? '?'))
  if (badPartners.length) throw new Error(`hq/site.toml: [[partners]] missing keys in ${badPartners.join(', ')}`)
  // [guards] is for the build checks only (scripts/check-dist.mjs): it must never ship in the page.
  const { guards: _guards, ...forPage } = cfg
  const shown = partners.filter((p) => p.show === true) as unknown as Record<string, unknown>
  return { ...forPage, partners: shown } as Record<string, Record<string, unknown>>
}

// Liquidity tab (F20): hq/liquidity.toml, the page's copy and sourced inputs. Every key must be present.
const LIQ_KEYS = {
  run: ['runs', 'custom_max_pct', 'fix_step_pct', 'fix_quick_runs', 'tier_a_pct', 'tier_b_pct', 'hidden_settings', 'hidden_assumptions'],
  copy: ['title', 'chart_title', 'settings_note', 'audit_note', 'revenue_note', 'link_label'],
} as const
const LIQ_ITEM_KEYS = { years: ['key', 'label', 'rate', 'tip'], switches: ['key', 'phrase', 'label', 'tip', 'on'] } as const

function liquidityConfig() {
  const cfg = parseToml(readFileSync(new URL('./hq/liquidity.toml', import.meta.url), 'utf8')) as Record<string, unknown>
  const table = (t: string) => (cfg[t] ?? {}) as Record<string, unknown>
  const missing = Object.entries(LIQ_KEYS).flatMap(([t, keys]) => keys.filter((k) => table(t)[k] == null).map((k) => `${t}.${k}`))
  for (const [list, keys] of Object.entries(LIQ_ITEM_KEYS)) {
    const items = (cfg[list] ?? []) as Record<string, unknown>[]
    if (!items.length) missing.push(`[[${list}]]`)
    items.forEach((it, i) => keys.filter((k) => it[k] == null).forEach((k) => missing.push(`${list}[${i}].${k}`)))
  }
  if (missing.length) throw new Error(`hq/liquidity.toml is missing: ${missing.join(', ')}`)
  return cfg
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
  // sim/engine has its own node:test suite (npm run test:engine), so vitest skips it.
  test: { exclude: [...configDefaults.exclude, 'sim/**'] },
  // HOT Wallet's SDK expects Node's global/Buffer -- see src/lib/client.tsx.
  plugins: [react(), nodePolyfills({ include: ['buffer'], globals: { Buffer: true, global: true } })],
  define: {
    'process.env.NODE_ENV': JSON.stringify(mode),
    __POOL__: JSON.stringify(poolConfig(mode)),
    __SITE__: JSON.stringify(site),
    __LIQ__: JSON.stringify(liquidityConfig()),
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
