// Mendel's fonts (as on example.com), shipped with the app: the webview makes no
// network requests.
import "@fontsource-variable/plus-jakarta-sans";
import "@fontsource/poppins/600.css";
import "@fontsource/poppins/700.css";
import "@fontsource-variable/jetbrains-mono";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const app = mount(App, { target: document.getElementById("app")! });

export default app;
