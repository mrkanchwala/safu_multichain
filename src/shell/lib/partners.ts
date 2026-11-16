import type { Partner } from "./site";

export interface PartnerRow {
  label: string;
  partners: Partner[];
}

/** Group partners into rows, keeping the order rows and partners first appear in hq/site.toml. */
export function groupPartners(partners: Partner[]): PartnerRow[] {
  const rows: PartnerRow[] = [];
  for (const p of partners) {
    const row = rows.find((r) => r.label === p.row);
    if (row) row.partners.push(p);
    else rows.push({ label: p.row, partners: [p] });
  }
  return rows;
}
