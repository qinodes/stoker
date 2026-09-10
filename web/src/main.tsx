import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import { App } from "./App";
import { WorkspaceProvider } from "./context";

const root = document.getElementById("root");
if (!root) throw new Error("Stoker UI root element is missing");

createRoot(root).render(<StrictMode><WorkspaceProvider><App /></WorkspaceProvider></StrictMode>);
