import { DetailsLayout } from "./DetailsLayout";
import { LINKS } from "../lib/links";

// Whitelabel (Model 1) page for partners. Same content as safustaking.com/protocols.html,
// rewritten for the multichain build and styled like the FAQ.
const SECTIONS: { id: string; label: string; title: string; intro: string; items: { t: string; d: string }[] }[] = [
  {
    id: "pr-platforms",
    label: "For platforms",
    title: "Protect your own users' wallets.",
    intro:
      "You hold your users' funds, or your users hold their own wallet through your product. A verified drain or wrongful liquidation triggers an automatic payout, up to a set multiple of the stake, sized to the wallet's risk tier and never more than the loss itself.",
    items: [
      {
        t: "Your pool, your brand.",
        d: "Your users stake through your product and the pool carries your name. The engine underneath is ours, so you ship coverage without building a claims system to run it.",
      },
      {
        t: "Wallet-level loss covered.",
        d: "Phishing, approval exploits, key compromise and wrongful liquidations are how users actually lose money, and the pool covers them whether a person or an autonomous agent runs the wallet.",
      },
      {
        t: "No vote, no claim form.",
        d: "The contract enforces the payout rule itself, and the same loss event always produces the same calculation. Nobody sits in a room deciding who deserves it.",
      },
      {
        t: "Nothing collected upfront.",
        d: "Your users pay nothing upfront. A staker who claims forfeits their principal, and that forfeiture is what funds the payout.",
      },
    ],
  },
  {
    id: "pr-ecosystems",
    label: "For ecosystems",
    title: "Protect the protocols in your ecosystem.",
    intro:
      "A chain or a foundation can also deploy one level up, with a pool that covers the protocols built on its ecosystem. It works like a reserve ratio: when a protocol in the pool is hacked, the payout goes to the protocol itself, deterministically and fast, so it can absorb the loss and keep honoring withdrawals. Their end-users are protected because the protocol stays solvent.",
    items: [
      {
        t: "Stops a bank run before it starts.",
        d: "The payout lands fast enough that the protocol keeps honoring withdrawals, so its users never see a locked balance.",
      },
      {
        t: "Protocol-level exploits covered.",
        d: "Oracle and governance manipulation: the attacks an audit looks for, covered for the day one slips through.",
      },
      {
        t: "Member protocols stake in too.",
        d: "Protocols in the pool stake into a shared reserve, with optional seed capital from the launching ecosystem. It runs on the same forfeiture-funded mechanism as the wallet-level pools, one level up.",
      },
      {
        t: "Isolated per deployment.",
        d: "Each pool is its own contract instance, capitalized independently. A new partner's pool never exposes another partner's stakers.",
      },
    ],
  },
  {
    id: "pr-chains",
    label: "Any chain",
    title: "Built once, ported per chain.",
    intro:
      "The same deterministic mechanism runs on Ethereum's EVM and Stellar's Soroban, and one pool on Stellar already covers wallets on Ethereum, Solana and Stellar through Circle's CCTP. Those are the proof that it ports, and more chains can follow.",
    items: [
      {
        t: "One engine for every chain.",
        d: "Scoring and payouts work the same way everywhere. A new chain is a new deployment of the same engine, or a new chain connected to an existing pool.",
      },
      {
        t: "A security review per chain.",
        d: "Every new deployment goes through its own security review before it takes real stakes.",
      },
      {
        t: "Bring us your ecosystem.",
        d: "Tell us which chain, and we scope what it takes to run a pool there, with no fixed roadmap promising a chain we haven't committed to yet.",
      },
    ],
  },
];

export function Protocols({ onBack }: { onBack: () => void }) {
  return (
    <DetailsLayout
      sections={[...SECTIONS.map((s) => ({ id: s.id, label: s.label })), { id: "pr-contact", label: "Get in touch" }]}
      cross={{ label: "Read the whitepaper →", href: "#whitepaper" }}
      onBack={onBack}
    >
      <div className="wp-head">
        <h1>Run this pool under your own name.</h1>
        <div className="sub-line">
          SAFU gives protocols, wallet providers and platforms holding user funds a way to protect their users,
          people or autonomous agents alike, from phishing, approval exploits, key compromise and wrongful
          liquidations. An ecosystem can deploy the same pool one level up and protect the protocols built on it
          instead: when one is hacked, the payout goes to the protocol itself, fast enough to stop a bank run
          before it starts. Neither flow has a vote or a claim form, because the contract enforces the payout rule
          itself.
        </div>
      </div>
      {SECTIONS.map((s) => (
        <div className="slide faq-group" id={s.id} key={s.id}>
          <h2>{s.title}</h2>
          <p>{s.intro}</p>
          {s.items.map((it) => (
            <div className="faq-item" key={it.t}>
              <h3>{it.t}</h3>
              <p>{it.d}</p>
            </div>
          ))}
        </div>
      ))}
      <div className="slide" id="pr-contact">
        <h2>Tell us about your pool.</h2>
        <p>
          We set up each whitelabel pool with the partner directly, and pricing and scope are agreed per partner.
          Reach out and we'll walk through what a pool under your name looks like.
        </p>
        <a className="cta-link" href={LINKS.bdIntake} target="_blank" rel="noopener noreferrer">
          Get in touch
        </a>
      </div>
    </DetailsLayout>
  );
}
