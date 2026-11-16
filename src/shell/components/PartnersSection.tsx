import { SITE } from "../lib/site";
import { groupPartners } from "../lib/partners";

// nofollow: partner links never pass our search ranking on (founder, 16 Nov).
const EXT = { target: "_blank", rel: "nofollow noopener noreferrer" } as const;

export function PartnersSection() {
  const rows = groupPartners(SITE.partners);
  if (rows.length === 0) return null;
  return (
    <section className="partners" id="partners">
      <div className="wrap">
        <h2>Backed and built with</h2>
        <div className="sub">The teams and networks behind SAFU.</div>
        <div className="partners-strip">
          {rows.map((row) => (
            <div className="partners-row" key={row.label}>
              <div className="partners-label">{row.label}</div>
              <div className="partners-items">
                {row.partners.map((p) => (
                  <a key={p.name} href={p.url} title={p.name} {...EXT}>
                    {p.logo ? <img src={p.logo} alt={p.name} /> : <span>{p.name}</span>}
                    {p.logo && p.logo_text ? <span>{p.logo_text}</span> : null}
                  </a>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
