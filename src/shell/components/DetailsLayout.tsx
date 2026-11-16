import type { ReactNode } from "react";

// Sidebar + slides layout shared by How it works and the Whitepaper. Section
// links scroll instead of setting the URL hash: the hash picks the page, so an
// anchor link would close it.
export function DetailsLayout({
  sections,
  cross,
  onBack,
  children,
}: {
  sections: { id: string; label: string }[];
  cross: { label: string; href: string };
  onBack: () => void;
  children: ReactNode;
}) {
  return (
    <div className="details-layout">
      <aside>
        <button className="back" onClick={onBack}>
          <img src="/logo.png" alt="" />← Back to app
        </button>
        <nav>
          {sections.map((s) => (
            <button key={s.id} onClick={() => document.getElementById(s.id)?.scrollIntoView({ behavior: "smooth" })}>
              {s.label}
            </button>
          ))}
        </nav>
        <a className="cross-link" href={cross.href}>
          {cross.label}
        </a>
      </aside>
      <main>{children}</main>
    </div>
  );
}
