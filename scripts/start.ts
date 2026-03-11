import { fileURLToPath } from "node:url";

const distDirectory = fileURLToPath(new URL("../dist/", import.meta.url));

process.chdir(distDirectory);

await import(new URL("../dist/server.js", import.meta.url).href);

export {};
