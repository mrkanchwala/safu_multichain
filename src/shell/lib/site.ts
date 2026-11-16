// The site's own values from hq/site.toml, baked in at build time (vite.config.ts checks every key).
// Read site values from here, never type a domain, link or chain list anywhere else.
declare const __SITE__: SiteConfig;

export interface SiteConfig {
  site: { name: string; url: string; icon: string; wallet_description: string; walletconnect_project_id: string };
  links: { feedback: string; x: string; telegram: string; github: string; bd_intake: string };
  chains: { enabled: string[]; default: string; labels: Record<string, string> };
}

export const SITE: SiteConfig = __SITE__;

/** What a wallet shows about the site when it connects (WalletConnect and the Stellar kit). */
export const WALLET_METADATA = {
  name: SITE.site.name,
  description: SITE.site.wallet_description,
  url: SITE.site.url,
  icons: [SITE.site.icon],
};
