import { DetailsLayout } from "./DetailsLayout";

// v1 whitepaper, the same for every pool and chain (founder 16 Nov): no chain, asset or vendor names.
// The ETH-era one stays on safustaking.com/whitepaper.html.
const SECTIONS = [
  { id: "wp-summary", label: "Summary" },
  { id: "wp-problem", label: "The problem" },
  { id: "wp-what", label: "What SAFU is" },
  { id: "wp-covered", label: "What's covered" },
  { id: "wp-tiers", label: "Coverage and tiers" },
  { id: "wp-clock", label: "The clock" },
  { id: "wp-chains", label: "Across chains" },
  { id: "wp-checks", label: "How a claim is checked" },
  { id: "wp-backing", label: "Backing the pool" },
  { id: "wp-yield", label: "Yield" },
  { id: "wp-security", label: "Security status" },
  { id: "wp-protocols", label: "For protocols" },
  { id: "wp-before", label: "Before you stake" },
  { id: "wp-legal", label: "Not insurance" },
];

function Slide({ id, title, children }: { id: string; title: string; children: React.ReactNode }) {
  return (
    <div className="slide" id={id}>
      <h2>{title}</h2>
      {children}
    </div>
  );
}

export function Whitepaper({ onBack }: { onBack: () => void }) {
  return (
    <DetailsLayout sections={SECTIONS} cross={{ label: "Read the FAQ →", href: "#faq" }} onBack={onBack}>
      <div className="wp-head">
        <h1>SAFU whitepaper</h1>
        <div className="wp-meta">v1 · November 2026</div>
      </div>

      <div className="wp">
        <Slide id="wp-summary" title="Summary">
          <p>
            SAFU is a shared pool that pays people back when crypto is stolen from their wallets or their loans are
            liquidated at a price that wasn't real. Stakers put money into a pool and register the wallets they want
            covered, on the pool's own chain or on other chains it supports. If crypto is stolen from one of those
            wallets, the pool pays automatically, up to 15 times the stake and capped at the real loss. The rules that
            decide a payout are written into the contract and the claim checks, and nobody votes on claims. Today SAFU
            pays back crypto stolen from wallets; cover for thefts from protocols comes next.
          </p>
        </Slide>

        <Slide id="wp-problem" title="The problem">
          <p>
            Every week, people have crypto stolen through phishing links, malicious token approvals and stolen keys. When it
            happens, the options are thin. A wallet provider or exchange might look at the case, or a committee might
            vote on it weeks later, and the rules they use are rarely published, so most people only learn whether
            they'll be paid long after the loss. Lending has the same gap: when a loan is liquidated on a price that
            was never real, the borrower usually absorbs the loss alone. SAFU exists so that the answer is known
            before the loss happens, with fixed rules published in advance and applied the same way to everyone.
          </p>
        </Slide>

        <Slide id="wp-what" title="What SAFU is">
          <p>
            A pool contract that holds the stakes and pays the claims. One pool can cover wallets on several chains,
            and the payout always comes from that pool, whichever chain the loss happened on. Anyone can stake within
            the pool's limits, and there is no whitelist.
          </p>
        </Slide>

        <Slide id="wp-covered" title="What's covered">
          <p>
            SAFU covers four kinds of loss, and all four go through the same claim path and the same ceilings. Each pool
            lists its chains and assets in its app.
          </p>
          <div className="table-scroll">
            <table className="mini-table">
              <tbody>
                <tr>
                  <th>Loss</th>
                  <th>What happened</th>
                </tr>
                <tr>
                  <td>Phishing</td>
                  <td>You signed a transaction on a fake site or app, and it sent your funds to an attacker.</td>
                </tr>
                <tr>
                  <td>Approval exploit</td>
                  <td>A malicious or abused token approval let someone else move your tokens.</td>
                </tr>
                <tr>
                  <td>Key compromise</td>
                  <td>Your private key or seed phrase was stolen and used to empty the wallet.</td>
                </tr>
                <tr>
                  <td>Wrongful liquidation</td>
                  <td>
                    A loan held in a covered wallet was liquidated at a price that wasn't real, checked against independent
                    market price history (the exact window and threshold stay private).
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </Slide>

        <Slide id="wp-tiers" title="Coverage and tiers">
          <p>Each covered wallet gets a tier from its own on-chain history. The tier sets a ceiling on the payout:</p>
          <div className="table-scroll">
            <table className="mini-table">
              <tbody>
                <tr>
                  <th>Tier</th>
                  <th>Ceiling</th>
                </tr>
                <tr>
                  <td>A</td>
                  <td>15x stake</td>
                </tr>
                <tr>
                  <td>B</td>
                  <td>10x stake</td>
                </tr>
                <tr>
                  <td>C</td>
                  <td>5x stake</td>
                </tr>
              </tbody>
            </table>
          </div>
          <p>
            The payout is the smaller of that ceiling and the real loss. The ceilings are set in the pool contract, and
            nobody can raise the ceiling on a stake that's already in. Each stake covers one claim, and that claim can
            bundle several theft transactions across the covered wallets. After it's paid, the stake that backed it is used up, even if the payout came in under the
            ceiling. A staking wallet that has had a claim paid can't stake again.
          </p>
        </Slide>

        <Slide id="wp-clock" title="The clock">
          <p>
            A loss has to be claimed within the pool's claim window. A claim on a new stake is recorded at its full amount
            and held until the stake is old enough. Nothing is taken while it waits. Once you
            approve a claim, a cooldown starts, and then the payout streams to you. A daily payout limit spreads large
            incidents out, so when many claims arrive at once they wait in a queue and are paid in order. Each pool
            shows its own windows in its app.
          </p>
        </Slide>

        <Slide id="wp-chains" title="Across chains">
          <p>
            You can stake from a wallet on a different chain from the pool. Your stake moves to the pool's chain over a
            standard stablecoin bridge, and SAFU pays the fees for that step. Payouts and withdrawals go back
            to the wallet you staked from, on its own chain, where your wallet signs one receive transaction. If you leave
            that last step unfinished, it can still be completed later.
          </p>
          <p>We recommend staking from a fresh, secure wallet, since the payout goes back to it.</p>
        </Slide>

        <Slide id="wp-checks" title="How a claim is checked">
          <p>
            Ownership comes first: a covered wallet must be registered before the theft, and you show it's yours by
            sending a tiny amount from that wallet to itself. You never sign a message with the covered wallet, because a wallet whose key was stolen would
            let the thief sign too.
          </p>
          <p>
            The scanner then reads each theft or liquidation transaction on its own chain and prices a theft with an on-chain
            price oracle, and a liquidation with independent market price history. If the claim checks out, the oracle signs the result. The
            oracle key sits in a cloud key-management service and never leaves it. The scoring details stay private,
            because publishing them would make the checks easier to game. The time rules above are public.
          </p>
          <div className="table-scroll">
            <table className="mini-table">
              <tbody>
                <tr>
                  <th>On-chain (in the contract)</th>
                  <th>Off-chain (checks, can be corrected)</th>
                </tr>
                <tr>
                  <td>Tier ceilings (15x / 10x / 5x)</td>
                  <td>Which wallets are registered to which staker</td>
                </tr>
                <tr>
                  <td>Rejecting any payout above the ceiling</td>
                  <td>Scanning the theft or liquidation transaction</td>
                </tr>
                <tr>
                  <td>Checking the oracle signature</td>
                  <td>Price reads and the wrongful-liquidation check</td>
                </tr>
                <tr>
                  <td>New-stake hold, cooldown, payout stream</td>
                  <td>The payout number itself</td>
                </tr>
                <tr>
                  <td>Solvency and daily payout limits</td>
                  <td>Capping at the real loss</td>
                </tr>
              </tbody>
            </table>
          </div>
          <p>A few actions, such as cancelling a claim found to be fraudulent, need two separate keys to sign.</p>
        </Slide>

        <Slide id="wp-backing" title="Backing the pool">
          <p>
            Backers add funds to help the pool pay large claims on time, without registering any wallets. Backing
            matures after a short period and can then be withdrawn. Backers always get back exactly what they put in,
            but they can only take out money that open claims don't need. Backers can come in from other chains too.
          </p>
        </Slide>

        <Slide id="wp-yield" title="Yield">
          <p>
            The pool keeps some cash on hand and can lend the rest on an established lending market, moving money back
            automatically when a withdrawal or payout needs it. By default the yield on your own stake goes to you, and
            backers get the yield on their matured backing.
          </p>
        </Slide>

        <Slide id="wp-security" title="Where the security work stands">
          <p>
            Each pool contract has its own test suites and fuzz testing, and design choices we already know about are
            written up for the auditors. Independent audits are being arranged, and the reports will be linked here.
          </p>
        </Slide>

        <Slide id="wp-protocols" title="For protocols">
          <p>
            Consumer dApps, exchanges and platforms that hold user funds can run a SAFU pool under their own name, for their
            own users.
          </p>
          <p className="inline-links">
            <a href="#protocols">For protocols</a>
          </p>
        </Slide>

        <Slide id="wp-before" title="Before you stake">
          <ul className="callout">
            <li>When a claim is paid, the stake behind it is used up.</li>
            <li>
              If the pool shrinks with no new money coming in, the last people to withdraw may wait until new stake or
              backing arrives.
            </li>
            <li>Payouts go back to the wallet you staked from, so stake from a wallet you keep safe.</li>
            
          </ul>
        </Slide>

        <Slide id="wp-legal" title="SAFU is not insurance">
          <ul>
            <li>There's no insurer and no promise from a company. The pool contract pays out by its own published rules.</li>
            <li>You pay no premium: you stake your own money and keep earning yield on it.</li>
            <li>No payout is guaranteed, because payouts are capped and paid only while the pool has the money.</li>
            <li>SAFU never holds your funds. They sit in the pool contract.</li>
          </ul>
        </Slide>
      </div>
    </DetailsLayout>
  );
}
