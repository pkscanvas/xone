// Bundled locally so headlines render identically offline; the opsz build
// lets the browser size the serif optically at each step of the scale.
import "@fontsource-variable/newsreader/opsz.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "@fontsource/ibm-plex-mono/latin-600.css";
import "@fontsource-variable/azeret-mono/wght.css";
import "./app.css";
import App from "./App.svelte";
import { mount } from "svelte";

const app = mount(App, {
  target: document.getElementById("app") as HTMLElement
});

export default app;
