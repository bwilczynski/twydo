import { render } from "preact";
import { App } from "./App";
import { createDataSource } from "./data";
import "./styles.css";

render(<App ds={createDataSource()} />, document.getElementById("app")!);

if ("serviceWorker" in navigator && import.meta.env.PROD) {
  navigator.serviceWorker.register("/sw.js").catch(() => {});
}
