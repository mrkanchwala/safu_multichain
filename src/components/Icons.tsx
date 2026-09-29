// X and Telegram marks, the same paths as the main safustaking.com footer.
export function XIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-4.714-6.231-5.401 6.231H2.744l7.737-8.835L1.254 2.25H8.08l4.261 5.632 5.903-5.632zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
    </svg>
  );
}

// Chain + WalletConnect marks for the Connect dropdown: simplified one-colour shapes (currentColor),
// same treatment as the X / Telegram marks, so they sit quietly in the site's dark palette.
export function EthereumIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">
      <path d="M12 1.5 5.25 12.2 12 16.1l6.75-3.9L12 1.5z" opacity="0.9" />
      <path d="M12 17.4 5.25 13.5 12 22.5l6.75-9L12 17.4z" opacity="0.6" />
    </svg>
  );
}

export function SolanaIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">
      <path d="M6.2 5h14.3l-2.7 3H3.5l2.7-3zM3.5 10.5h14.3l2.7 3H6.2l-2.7-3zM6.2 16h14.3l-2.7 3H3.5l2.7-3z" />
    </svg>
  );
}

export function StellarIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M18.6 6.2A8 8 0 0 0 4.3 14.6" />
      <path d="M5.4 17.8a8 8 0 0 0 14.3-8.4" />
      <path d="M2.5 15.6 21.5 6.9M2.5 17.3l19-8.7" />
    </svg>
  );
}

export function WalletConnectIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">
      <path d="M6.1 8.6a8.4 8.4 0 0 1 11.8 0l.4.4a.4.4 0 0 1 0 .6l-1.3 1.3a.2.2 0 0 1-.3 0l-.5-.6a5.9 5.9 0 0 0-8.3 0l-.6.6a.2.2 0 0 1-.3 0L5.7 9.6a.4.4 0 0 1 0-.6l.4-.4zm14.5 2.7 1.2 1.2a.4.4 0 0 1 0 .6l-5.3 5.2a.4.4 0 0 1-.6 0L12.2 14.6a.1.1 0 0 0-.2 0l-3.7 3.7a.4.4 0 0 1-.6 0L2.2 13.1a.4.4 0 0 1 0-.6l1.2-1.2a.4.4 0 0 1 .6 0l3.7 3.7a.1.1 0 0 0 .2 0l3.7-3.7a.4.4 0 0 1 .6 0l3.7 3.7a.1.1 0 0 0 .2 0l3.7-3.7a.4.4 0 0 1 .6 0z" />
    </svg>
  );
}

export function TelegramIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
    </svg>
  );
}
