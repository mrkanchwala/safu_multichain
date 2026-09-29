export function AboutSection() {
  return (
    <section className="about" id="about">
      <div className="wrap about-layout">
        <div className="about-head">
          <h2>What is SAFU?</h2>
          <div className="sub">The short version, and why we built it.</div>
        </div>
        <div className="about-body">
          <p>
            <b>Why this exists.</b> Wallets get drained every week through phishing, bad approvals and stolen
            keys, and loans get liquidated on prices that were never real. The people it happens to usually wait
            weeks for a support team or a committee to decide whether they deserve help, under rules they never got
            to see, and many get nothing. We built SAFU so that whether you get paid back comes from rules you can
            read before anything goes wrong.
          </p>
          <p>
            <b>How it pays.</b> You put USDC into one shared pool on Stellar and register up to three wallets you care
            about, on Ethereum, Solana or Stellar. If one of them gets drained through phishing, a bad approval or a
            stolen key, or a loan in it is liquidated at a price that wasn't real, you file a claim and fixed rules work out the payout: up to 15 times your stake depending on the
            wallet's tier, and never more than you actually lost. It's paid in USDC, back to the wallet you staked from.
          </p>
          <p>
            <b>How claims get checked.</b> Our scanner reads the drain transaction straight from the chain it happened
            on and decides whether it looks like a real drain. The same transaction gets the same answer every time, and
            the pool only pays when that answer carries the oracle's signature, so there's no vote and nobody picks
            favourites.
          </p>
        </div>
      </div>
    </section>
  );
}
