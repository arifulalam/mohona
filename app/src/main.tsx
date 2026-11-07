import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { DropWindow } from "./DropWindow";
import { chooseApi } from "./api";
import { applyTheme, loadTheme } from "./theme";
import "./index.css";

applyTheme(loadTheme());
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {location.hash === "#drop" ? <DropWindow api={chooseApi()} /> : <App api={chooseApi()} />}
  </StrictMode>,
);
