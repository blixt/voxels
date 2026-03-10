import { buildClientBundles } from "../src/server/build-client.ts";

await buildClientBundles();
console.log("Built browser bundles into public/build");
