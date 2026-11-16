import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./shell/index.css";
import App from "./App.tsx";
import { loadAdapter } from "./chains/registry";

loadAdapter(window.location.search).then((adapter) => {
  const { Provider } = adapter;
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Provider>
        <App adapter={adapter} />
      </Provider>
    </StrictMode>,
  );
});
