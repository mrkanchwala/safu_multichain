import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./shell/index.css";
import App from "./App.tsx";
import { loadAdapter } from "./chains/registry";

const root = createRoot(document.getElementById("root")!);

// The pool's code is its own chunk: a failed fetch (network blip, chunk replaced by a deploy)
// must show a way out, never a blank page.
Promise.resolve()
  .then(() => loadAdapter(window.location.search))
  .then((adapter) => {
    const { Provider } = adapter;
    root.render(
      <StrictMode>
        <Provider>
          <App adapter={adapter} />
        </Provider>
      </StrictMode>,
    );
  })
  .catch(() => {
    root.render(
      <div className="devnet-badge">
        The page didn't load. <a href={window.location.href}>Reload</a>
      </div>,
    );
  });
