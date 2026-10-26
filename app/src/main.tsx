import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { chooseApi } from "./api";
import { applyTheme, loadTheme } from "./theme";
import "./index.css";

applyTheme(loadTheme());
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App api={chooseApi()} />
  </StrictMode>,
);
