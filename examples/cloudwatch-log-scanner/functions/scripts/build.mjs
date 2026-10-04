import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { buildFunctionArtifacts } from "../../../../functions/build-function-artifacts.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

await buildFunctionArtifacts({ root });
