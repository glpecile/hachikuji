// Build step (Bun): reads each withfig/autocomplete spec WITHOUT running it. The TypeScript
// parser gives the syntax tree, and a small evaluator keeps only literal values: objects,
// arrays, strings, numbers, booleans, constants (local or imported from other spec files),
// spreads and simple `[...].map(x => ({...}))`. Functions are never called; they are dropped
// and listed. Users never run this.
//
//   bun tools/extract.ts <fig-repo> <typescript-module-dir> <out-dir> <lost-file>
//
// A generator that needs JS (postProcess, custom) is dropped and listed in <lost-file>, unless it
// maps to a built-in (BUILTINS, gitBuiltin) or to the filepaths()/folders() helpers.

import { readFileSync, readdirSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import { join, relative, dirname, resolve } from "node:path";

const [repo, tsDir, out, lostFile] = process.argv.slice(2).map((p) => resolve(p));
const ts = createRequire(join(tsDir, "package.json"))("typescript");

const FUNC = { __func: true }; // a function value: never run
const UNKNOWN = undefined;

type Mod = { decls: Map<string, any>; imports: Map<string, { from: string; name: string }>; values: Map<string, any>; file: string; def?: any };
const mods = new Map<string, Mod>();

function load(file: string): Mod | undefined {
  if (mods.has(file)) return mods.get(file);
  if (!existsSync(file)) return;
  const sf = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
  const m: Mod = { decls: new Map(), imports: new Map(), values: new Map(), file };
  mods.set(file, m);
  for (const st of sf.statements) {
    if (ts.isImportDeclaration(st) && st.importClause) {
      const from = st.moduleSpecifier.text;
      const nb = st.importClause.namedBindings;
      if (st.importClause.name) m.imports.set(st.importClause.name.text, { from, name: "default" });
      if (nb && ts.isNamedImports(nb))
        for (const e of nb.elements) m.imports.set(e.name.text, { from, name: (e.propertyName ?? e.name).text });
    } else if (ts.isVariableStatement(st)) {
      for (const d of st.declarationList.declarations)
        if (ts.isIdentifier(d.name) && d.initializer) m.decls.set(d.name.text, d.initializer);
    } else if (ts.isFunctionDeclaration(st) && st.name) {
      m.decls.set(st.name.text, "function");
    } else if (ts.isExportAssignment(st)) {
      m.def = st.expression;
    } else if (ts.isExportDeclaration(st) && st.exportClause && ts.isNamedExports(st.exportClause)) {
      for (const e of st.exportClause.elements)
        if ((e.name.text === "default") && e.propertyName) m.def = ts.factory.createIdentifier(e.propertyName.text);
    }
  }
  return m;
}

function resolveImport(m: Mod, from: string): string | undefined {
  if (!from.startsWith(".")) return;
  const base = resolve(dirname(m.file), from);
  return [`${base}.ts`, join(base, "index.ts")].find(existsSync);
}

function lookup(m: Mod, name: string, depth: number): any {
  if (m.values.has(name)) return m.values.get(name);
  m.values.set(name, UNKNOWN); // cycle guard
  let v: any = UNKNOWN;
  const d = m.decls.get(name);
  if (d === "function") v = FUNC;
  else if (d) v = evaluate(d, m, new Map(), depth + 1);
  else if (m.imports.has(name)) {
    const imp = m.imports.get(name)!;
    if (imp.from === "@fig/autocomplete-generators" && (imp.name === "filepaths" || imp.name === "folders"))
      v = { __helper: imp.name };
    else {
      const f = resolveImport(m, imp.from);
      const t = f && load(f);
      if (t) v = imp.name === "default" ? (t.def ? evaluate(t.def, t, new Map(), depth + 1) : UNKNOWN) : lookup(t, imp.name, depth + 1);
    }
  }
  // Name declared generators ("ssh.ts:knownHosts"), so BUILTINS can map them. clean() drops __id.
  if (d && d !== "function" && v && typeof v === "object" && !Array.isArray(v) && v !== FUNC && !v.__id)
    v.__id = `${relative(join(repo, "src"), m.file)}:${name}`;
  m.values.set(name, v);
  return v;
}

function propName(n: any): string | undefined {
  if (ts.isIdentifier(n) || ts.isStringLiteral(n) || ts.isNumericLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
}

function evaluate(n: any, m: Mod, scope: Map<string, any>, depth: number): any {
  if (depth > 60) return UNKNOWN;
  const ev = (x: any) => evaluate(x, m, scope, depth + 1);
  if (ts.isParenthesizedExpression(n) || ts.isAsExpression(n) || ts.isSatisfiesExpression?.(n) || ts.isTypeAssertionExpression(n) || ts.isNonNullExpression(n))
    return ev(n.expression);
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
  if (ts.isNumericLiteral(n)) return Number(n.text);
  if (ts.isPrefixUnaryExpression(n) && n.operator === ts.SyntaxKind.MinusToken) {
    const v = ev(n.operand);
    return typeof v === "number" ? -v : UNKNOWN;
  }
  if (n.kind === ts.SyntaxKind.TrueKeyword) return true;
  if (n.kind === ts.SyntaxKind.FalseKeyword) return false;
  if (n.kind === ts.SyntaxKind.NullKeyword) return null;
  if (ts.isArrowFunction(n) || ts.isFunctionExpression(n)) return FUNC;
  if (ts.isIdentifier(n)) return scope.has(n.text) ? scope.get(n.text) : n.text === "undefined" ? UNKNOWN : lookup(m, n.text, depth);
  if (ts.isTemplateExpression(n)) {
    let s = n.head.text;
    for (const sp of n.templateSpans) {
      const v = ev(sp.expression);
      if (typeof v !== "string" && typeof v !== "number") return UNKNOWN;
      s += v + sp.literal.text;
    }
    return s;
  }
  if (ts.isBinaryExpression(n) && n.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const a = ev(n.left), b = ev(n.right);
    return (typeof a === "string" || typeof a === "number") && (typeof b === "string" || typeof b === "number") ? a + b : UNKNOWN;
  }
  if (ts.isArrayLiteralExpression(n)) {
    const out: any[] = [];
    for (const e of n.elements) {
      if (ts.isSpreadElement(e)) {
        const v = ev(e.expression);
        if (Array.isArray(v)) out.push(...v);
      } else out.push(ev(e));
    }
    return out;
  }
  if (ts.isObjectLiteralExpression(n)) {
    const o: any = {};
    for (const p of n.properties) {
      if (ts.isPropertyAssignment(p)) {
        const k = propName(p.name);
        if (k !== undefined) o[k] = ev(p.initializer);
      } else if (ts.isShorthandPropertyAssignment(p)) o[p.name.text] = ev(p.name);
      else if (ts.isSpreadAssignment(p)) {
        const v = ev(p.expression);
        if (v && typeof v === "object" && !Array.isArray(v)) Object.assign(o, v);
      } else if (ts.isMethodDeclaration(p) || ts.isGetAccessorDeclaration(p)) {
        const k = propName(p.name);
        if (k !== undefined) o[k] = FUNC;
      }
    }
    return o;
  }
  if (ts.isPropertyAccessExpression(n)) {
    const v = ev(n.expression);
    return v && typeof v === "object" && v !== FUNC ? v[n.name.text] : UNKNOWN;
  }
  if (ts.isElementAccessExpression(n)) {
    const v = ev(n.expression), k = ev(n.argumentExpression);
    return v && typeof v === "object" && (typeof k === "string" || typeof k === "number") ? v[k] : UNKNOWN;
  }
  if (ts.isCallExpression(n)) {
    const callee = ev(n.expression);
    if (callee && callee.__helper) {
      // filepaths({ showFolders: "only" }) lists folders only (the cd spec). extensions and
      // equals keep only matching files (python: .py), plus folders.
      const opts = n.arguments.length ? ev(n.arguments[0]) : undefined;
      const t: any = { __template: opts?.showFolders === "only" ? "folders" : callee.__helper };
      const strs = (x: any) => (Array.isArray(x) ? x : [x]).filter((y) => typeof y === "string");
      if (opts?.extensions) t.extensions = strs(opts.extensions);
      if (opts?.equals) t.equals = strs(opts.equals);
      return t;
    }
    // [...].map((x) => ({ ... })) with a one-parameter arrow and an expression body.
    if (ts.isPropertyAccessExpression(n.expression) && n.expression.name.text === "map" && n.arguments.length === 1) {
      const arr = ev(n.expression.expression), f = n.arguments[0];
      if (Array.isArray(arr) && ts.isArrowFunction(f) && f.parameters.length >= 1 && ts.isIdentifier(f.parameters[0].name) && !ts.isBlock(f.body)) {
        const p = f.parameters[0].name.text;
        return arr.map((x) => evaluate(f.body, m, new Map([...scope, [p, x]]), depth + 1));
      }
    }
    return FUNC; // never run a call: its result counts as JS (often a postProcess function)
  }
  return UNKNOWN;
}

// --- from the evaluated value to the JSON the Rust compiler reads -------------------------

const KEEP = new Set([
  "name", "description", "subcommands", "options", "args", "isRepeatable", "exclusiveOn",
  "isPersistent", "isVariadic", "isOptional", "requiresSeparator", "requiresEquals",
  "requiresArgument", "insertValue", "hidden", "priority", "isCommand", "loadSpec",
  "template", "suggestions", "generators", "type",
]);

// Generators that need JS, by declared name, mapped to a built-in of the helper.
const BUILTINS: Record<string, string> = {
  "npm.ts:npmScriptsGenerator": "npm-scripts",
  "make.ts:listTargets": "make-targets",
  "ssh.ts:knownHosts": "ssh-hosts",
  "ssh.ts:configHosts": "ssh-hosts",
};

// Specs built by generateSpec (all JS): a static replacement.
const OVERRIDES: Record<string, any> = {
  z: { args: { name: "directory", generators: [{ __template: "folders" }, { builtin: "zoxide" }] } },
};

function gitBuiltin(script: unknown): string | undefined {
  const s = Array.isArray(script) ? script.join(" ") : typeof script === "string" ? script : "";
  if (!/^git\b/.test(s)) return;
  if (/\bstatus\b/.test(s)) return "git-changed";
  if (/\bfor-each-ref\b|\bbranch\b/.test(s)) return "git-branches";
  if (/\bremote\b/.test(s)) return "git-remotes";
  if (/\btag\b/.test(s)) return "git-tags";
}

const isFunc = (x: any) => x === FUNC;

function generator(g: any, lost: Set<string>): any {
  if (!g || typeof g !== "object" || isFunc(g)) return;
  if (g.__template) {
    const { __template, extensions, equals } = g;
    return { template: __template, ...(extensions && { extensions }), ...(equals && { equals }) };
  }
  if (typeof g.builtin === "string") return { builtin: g.builtin };
  if (BUILTINS[g.__id]) return { builtin: BUILTINS[g.__id] };
  const out: any = {};
  if (typeof g.template === "string" || Array.isArray(g.template)) out.template = g.template;
  if (isFunc(g.script)) lost.add("generator script function");
  else if (typeof g.script === "string" || Array.isArray(g.script)) {
    if (isFunc(g.postProcess) || isFunc(g.custom)) {
      const b = gitBuiltin(g.script);
      if (b) out.builtin = b;
      else lost.add("generator postProcess");
    } else {
      out.script = g.script;
      if (typeof g.splitOn === "string") out.splitOn = g.splitOn;
    }
  } else if (isFunc(g.custom) && !out.template) lost.add("generator custom");
  else if (g.script === undefined && g.custom === undefined && !out.template) lost.add("generator (not static)");
  return Object.keys(out).length ? out : undefined;
}

function clean(v: any, lost: Set<string>, key = ""): any {
  if (isFunc(v)) {
    if (key) lost.add(key);
    return;
  }
  if (Array.isArray(v)) return v.map((x) => clean(x, lost)).filter((x) => x !== undefined);
  if (!v || typeof v !== "object") return v;
  const o: any = {};
  for (const [k, x] of Object.entries(v)) {
    if (k === "generators") {
      const gs = (Array.isArray(x) ? x : [x]).map((g) => generator(g, lost)).filter(Boolean);
      if (gs.length) o.generators = gs;
    } else if (k === "loadSpec" && typeof x !== "string") {
      if (x !== undefined) lost.add("loadSpec (not a name)");
    } else if (KEEP.has(k)) {
      const c = clean(x, lost, k);
      if (c !== undefined) o[k] = c;
    } else if (isFunc(x)) lost.add(k); // generateSpec, parserDirectives, ...
  }
  return o;
}

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : e.name.endsWith(".ts") && !e.name.endsWith(".d.ts") ? [join(dir, e.name)] : [],
  );
}

const src = join(repo, "src");
const lostLines: string[] = [];
let n = 0;
for (const f of files(src).sort()) {
  const name = relative(src, f).replace(/\.ts$/, "");
  const lost = new Set<string>();
  const m = load(f)!;
  if (!m.def) continue; // a helper file, not a spec
  const spec = evaluate(m.def, m, new Map(), 0);
  if (!spec || typeof spec !== "object" || Array.isArray(spec) || !spec.name) {
    lostLines.push(`${name}: not static (whole spec)`);
    continue;
  }
  if (OVERRIDES[name]) Object.assign(spec, OVERRIDES[name]);
  const json = clean(spec, lost);
  mkdirSync(dirname(join(out, name)), { recursive: true });
  writeFileSync(join(out, `${name}.json`), JSON.stringify(json));
  n++;
  if (lost.size) lostLines.push(`${name}: ${[...lost].sort().join(", ")}`);
}
writeFileSync(lostFile, lostLines.join("\n") + "\n");
console.log(`specs=${n} with-losses=${lostLines.length}`);
