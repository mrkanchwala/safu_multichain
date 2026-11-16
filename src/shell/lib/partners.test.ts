import { describe, expect, it } from "vitest";
import { groupPartners } from "./partners";

const p = (row: string, name: string) => ({ row, name, url: "u", logo: "", logo_text: "" });

describe("groupPartners", () => {
  it("keeps first-seen row order and partner order", () => {
    const rows = groupPartners([p("Funded by", "A"), p("Built on", "B"), p("Funded by", "C")]);
    expect(rows.map((r) => r.label)).toEqual(["Funded by", "Built on"]);
    expect(rows[0].partners.map((x) => x.name)).toEqual(["A", "C"]);
  });

  it("returns no rows for no partners", () => {
    expect(groupPartners([])).toEqual([]);
  });
});
