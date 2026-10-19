import { createRequire } from "node:module";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const functionsRoot = dirname(fileURLToPath(import.meta.url));

export async function buildFunctionArtifacts({ root, functions }) {
  const packageJson = await readPackageJson(root);
  const functionDefinitions = functions ?? packageJson.relayfold?.functions;
  validateFunctionDefinitions(functionDefinitions);

  const distDir = join(root, "dist");
  const packageDependencies = packageDependenciesFrom(packageJson);
  await rm(distDir, { recursive: true, force: true });
  await mkdir(distDir, { recursive: true });

  for (const def of functionDefinitions) {
    const code = await readFunctionCode(root, def.source);
    const dependencies = def.dependencies ?? packageDependencies;
    const functionDef = {
      id: def.id,
      dependencies,
      code: code.trimEnd(),
    };
    const dependencyYaml = dependencies.length === 0
      ? ["dependencies: []"]
      : ["dependencies:", ...dependencyLines(dependencies)];
    const yaml = [`id: ${JSON.stringify(def.id)}`, ...dependencyYaml, "code: |", indent(code.trimEnd(), 2), ""].join("\n");
    await writeFile(join(distDir, `${def.id}.yaml`), yaml, "utf8");
    await writeFile(join(distDir, `${def.id}.json`), `${JSON.stringify(functionDef, null, 2)}\n`, "utf8");
  }
}

async function readPackageJson(root) {
  return JSON.parse(await readFile(join(root, "package.json"), "utf8"));
}

function packageDependenciesFrom(packageJson) {
  return Object.entries(packageJson.dependencies ?? {})
    .map(([name, version]) => ({ name, version }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

function validateFunctionDefinitions(functionDefinitions) {
  if (!Array.isArray(functionDefinitions) || functionDefinitions.length === 0) {
    throw new Error(
      'Define at least one Function in package.json under "relayfold.functions".',
    );
  }

  const ids = new Set();
  for (const definition of functionDefinitions) {
    if (typeof definition?.id !== "string" || definition.id.length === 0) {
      throw new Error("Each Function definition must have a non-empty id.");
    }
    if (typeof definition.source !== "string" || definition.source.length === 0) {
      throw new Error(`Function "${definition.id}" must have a non-empty source path.`);
    }
    if (ids.has(definition.id)) {
      throw new Error(`Function id "${definition.id}" is defined more than once.`);
    }
    ids.add(definition.id);
  }
}

async function readFunctionCode(root, source) {
  const sourcePath = join(root, source);
  if (extname(sourcePath) !== ".ts") {
    return readFile(sourcePath, "utf8");
  }

  return compileTypeScript(root, sourcePath);
}

async function compileTypeScript(root, sourcePath) {
  const ts = loadTypeScript(root);
  const outDir = await mkdtemp(join(tmpdir(), "relayfold-functions-ts-"));

  try {
    const options = {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ES2022,
      moduleResolution: ts.ModuleResolutionKind.Bundler,
      lib: ["lib.es2022.d.ts", "lib.dom.d.ts"],
      types: ["node"],
      typeRoots: typeRoots(root),
      strict: true,
      skipLibCheck: true,
      esModuleInterop: true,
      allowSyntheticDefaultImports: true,
      declaration: false,
      sourceMap: false,
      removeComments: false,
      outDir,
      rootDir: root,
      noEmitOnError: true,
    };
    const host = ts.createCompilerHost(options);
    const program = ts.createProgram([sourcePath, ...await declarationFiles(root)], options, host);
    const emit = program.emit();
    const diagnostics = ts
      .getPreEmitDiagnostics(program)
      .concat(emit.diagnostics);

    if (diagnostics.length > 0) {
      throw new Error(formatDiagnostics(ts, diagnostics));
    }

    const emittedPath = join(outDir, relative(root, sourcePath)).replace(/\.ts$/, ".js");
    return readFile(emittedPath, "utf8");
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

async function declarationFiles(root) {
  const files = [];
  await collectDeclarationFiles(join(root, "src"), files);
  return files;
}

async function collectDeclarationFiles(dir, files) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }

  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      await collectDeclarationFiles(path, files);
    } else if (entry.isFile() && path.endsWith(".d.ts")) {
      files.push(path);
    }
  }
}

function loadTypeScript(root) {
  const packageRequire = createRequire(join(root, "package.json"));
  try {
    return packageRequire("typescript");
  } catch {
    const require = createRequire(import.meta.url);
    return require("../worker/node_modules/typescript");
  }
}

function typeRoots(root) {
  return [
    join(root, "node_modules", "@types"),
    join(functionsRoot, "..", "worker", "node_modules", "@types"),
  ];
}

function formatDiagnostics(ts, diagnostics) {
  return ts.formatDiagnosticsWithColorAndContext(diagnostics, {
    getCanonicalFileName: (fileName) => fileName,
    getCurrentDirectory: () => functionsRoot,
    getNewLine: () => "\n",
  });
}

function dependencyLines(dependencies) {
  return dependencies.flatMap((dependency) => [
    `  - name: ${JSON.stringify(dependency.name)}`,
    `    version: ${JSON.stringify(dependency.version)}`,
  ]);
}

function indent(value, spaces) {
  const prefix = " ".repeat(spaces);
  return value.split("\n").map((line) => `${prefix}${line}`).join("\n");
}
