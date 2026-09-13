import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import { App } from "./App";
import { WorkspaceProvider } from "./context";
import { I18nProvider } from "./i18n/context";

const root = document.getElementById("root");
if (!root) throw new Error("Stoker UI root element is missing");

createRoot(root).render(<StrictMode><I18nProvider><WorkspaceProvider><App /></WorkspaceProvider></I18nProvider></StrictMode>);
