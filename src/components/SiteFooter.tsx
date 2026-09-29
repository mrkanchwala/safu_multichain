import { TelegramIcon, XIcon } from "./Icons";
import { LINKS } from "../lib/links";

const EXT = { target: "_blank", rel: "noopener noreferrer" } as const;

export function SiteFooter() {
  return (
    <footer>
      <div className="wrap">
        <div className="footer-brand">
          <p>
            <img src="/logo.png" alt="" style={{ height: 16, verticalAlign: -3, marginRight: 6 }} />
            Built by SAFU, the same deterministic, no-vote protocol already live on Ethereum.
          </p>
          <div className="social">
            <a href={LINKS.x} aria-label="SAFU on X" title="X" {...EXT}>
              <XIcon />
            </a>
            <a href={LINKS.telegram} aria-label="SAFU on Telegram" title="Telegram" {...EXT}>
              <TelegramIcon />
            </a>
          </div>
        </div>
        <div className="footer-cols">
          <div>
            <h4>Learn</h4>
            <a href="#faq">FAQ</a>
            <a href="#whitepaper">Whitepaper</a>
            <a href="#about">About</a>
          </div>
          <div>
            <h4>SAFU</h4>
            <a href="#protocols">For protocols</a>
            <a href={LINKS.site} {...EXT}>
              safustaking.com
            </a>
            <a href={LINKS.github} {...EXT}>
              GitHub
            </a>
          </div>
          <div>
            <h4>Community</h4>
            <a href={LINKS.x} {...EXT}>
              X
            </a>
            <a href={LINKS.telegram} {...EXT}>
              Telegram
            </a>
            <a href={LINKS.feedback} {...EXT}>
              Feedback
            </a>
          </div>
        </div>
      </div>
    </footer>
  );
}
