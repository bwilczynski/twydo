import { render } from "preact";
import { App } from "./App";
import { createDataSource } from "./data";
import { browserStorage, createPush } from "./push";
import "./styles.css";

const registration =
  "serviceWorker" in navigator && import.meta.env.PROD
    ? navigator.serviceWorker.register("/sw.js").then(() => navigator.serviceWorker.ready)
    : undefined;
registration?.catch(() => {});

const push = createPush({
  fetch: (...a) => fetch(...a),
  registration: registration && "PushManager" in window ? registration : undefined,
  notification: "Notification" in window ? Notification : undefined,
  standalone:
    (navigator as Navigator & { standalone?: boolean }).standalone === true ||
    matchMedia("(display-mode: standalone)").matches,
  storage: browserStorage(),
});

render(<App ds={createDataSource()} push={push} />, document.getElementById("app")!);
