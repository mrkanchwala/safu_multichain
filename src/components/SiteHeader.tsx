import { useEffect, useRef, useState } from "react";
import { WalletButton } from "./WalletButton";
import { TelegramIcon, XIcon } from "./Icons";
import { LINKS } from "../lib/links";
import type { View } from "../lib/links";
import type { AppClient } from "../lib/client";

const NAV: { label: string; href: string; view?: View; external?: boolean }[] = [
  { label: "FAQ", href: "#faq", view: "faq" },
  { label: "Whitepaper", href: "#whitepaper", view: "whitepaper" },
  { label: "About", href: "#about" },
  { label: "For protocols", href: "#protocols", view: "protocols" },
  { label: "Feedback", href: LINKS.feedback, external: true },
];

const EXT = { target: "_blank", rel: "noopener noreferrer" } as const;

export function SiteHeader({ client, view }: { client: AppClient; view: View }) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close the small-screen menu on any click outside it.
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  const links = NAV.map((n) => (
    <a
      key={n.label}
      href={n.href}
      className={n.view && n.view === view ? "active" : undefined}
      onClick={() => setMenuOpen(false)}
      {...(n.external ? EXT : {})}
    >
      {n.label}
    </a>
  ));

  return (
    <header>
      <div className="wrap">
        <a className="logo" href="#" aria-label="SAFU Staking home">
          <img src="/logo.png" alt="SAFU" />
          <span className="name">SAFU Staking</span>
          <span className="fam">Multichain</span>
        </a>
        <div className="header-right">
          <nav className="site-nav">{links}</nav>
          <div className="nav-menu" ref={menuRef}>
            <button className="secondary-action" aria-expanded={menuOpen} onClick={() => setMenuOpen((v) => !v)}>
              Menu
            </button>
            {menuOpen ? <div className="wallet-dropdown nav-dropdown">{links}</div> : null}
          </div>
          <div className="social">
            <a href={LINKS.x} aria-label="SAFU on X" title="X" {...EXT}>
              <XIcon />
            </a>
            <a href={LINKS.telegram} aria-label="SAFU on Telegram" title="Telegram" {...EXT}>
              <TelegramIcon />
            </a>
          </div>
          <WalletButton client={client} />
        </div>
      </div>
    </header>
  );
}
