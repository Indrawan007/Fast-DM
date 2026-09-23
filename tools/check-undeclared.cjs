#!/usr/bin/env node
// Pemeriksa identifier tak-terdeklarasi untuk extension MV3.
//
// `node --check` (tests/extension_lint.cjs) hanya memvalidasi sintaks. Regresi
// v3.2.2 memakai `videoMenu` tanpa pernah mendeklarasikannya; handler-nya
// `async`, jadi ReferenceError menjadi unhandled rejection dan KETIGA item
// context menu mati. Alat ini menutup kelas bug itu secara langsung:
// dua lintasan AST (acorn) — (1) kumpulkan binding per scope, (2) selesaikan
// setiap referensi. Identifier yang tidak ada di rantai scope dan bukan global
// browser/JS = merah.
//
// Self-test tertanam (fixture `videoMenu`) supaya checker yang "lolos semua"
// karena tidak pernah melaporkan apa pun ikut gagal, bukan hanya berkas nyata.

"use strict";

const { existsSync, readFileSync } = require("node:fs");
const { join } = require("node:path");

let acorn;
try {
  acorn = require("acorn");
} catch (err) {
  console.error(
    "acorn tidak ditemukan. Jalankan `npm ci` di root repo (devDependency).",
  );
  console.error(err.message);
  process.exit(1);
}

const ROOT = join(__dirname, "..");
const FILES = [
  "extension/background.js",
  "extension/content.js",
  "extension/sniffer.js",
  "extension/popup.js",
];

// Global yang sah di service worker / content script / popup Chromium.
// Bukan daftar lengkap web platform — cukup yang extension ini pakai plus
// bawaan JS dan API browser yang wajar, supaya penambahan kode tidak
// langsung merah hanya karena `fetch` atau `crypto`.
const GLOBALS = new Set([
  "AggregateError",
  "Array",
  "ArrayBuffer",
  "Atomics",
  "BigInt",
  "BigInt64Array",
  "BigUint64Array",
  "Blob",
  "Boolean",
  "BroadcastChannel",
  "CSS",
  "CustomEvent",
  "DOMException",
  "DOMParser",
  "DataView",
  "Date",
  "Error",
  "EvalError",
  "Event",
  "EventTarget",
  "File",
  "FileReader",
  "FinalizationRegistry",
  "Float32Array",
  "Float64Array",
  "FormData",
  "Function",
  "Headers",
  "HTMLElement",
  "Image",
  "Infinity",
  "Int16Array",
  "Int32Array",
  "Int8Array",
  "Intl",
  "JSON",
  "Map",
  "Math",
  "MediaSource",
  "MutationObserver",
  "NaN",
  "Number",
  "Object",
  "Promise",
  "Proxy",
  "RangeError",
  "ReadableStream",
  "ReferenceError",
  "Reflect",
  "RegExp",
  "Request",
  "Response",
  "Set",
  "SharedArrayBuffer",
  "String",
  "Symbol",
  "SyntaxError",
  "TextDecoder",
  "TextEncoder",
  "TypeError",
  "URIError",
  "URL",
  "URLSearchParams",
  "Uint16Array",
  "Uint32Array",
  "Uint8Array",
  "Uint8ClampedArray",
  "WeakMap",
  "WeakRef",
  "WeakSet",
  "WebSocket",
  "Worker",
  "WritableStream",
  "XMLHttpRequest",
  "AbortController",
  "AbortSignal",
  "alert",
  "atob",
  "btoa",
  "browser",
  "caches",
  "cancelAnimationFrame",
  "chrome",
  "clearInterval",
  "clearTimeout",
  "clients",
  "confirm",
  "console",
  "crypto",
  "decodeURI",
  "decodeURIComponent",
  "document",
  "encodeURI",
  "encodeURIComponent",
  "escape",
  "eval",
  "fetch",
  "frames",
  "getComputedStyle",
  "getSelection",
  "globalThis",
  "history",
  "importScripts",
  "indexedDB",
  "isFinite",
  "isNaN",
  "localStorage",
  "location",
  "matchMedia",
  "navigator",
  "origin",
  "parent",
  "parseFloat",
  "parseInt",
  "performance",
  "prompt",
  "queueMicrotask",
  "registration",
  "requestAnimationFrame",
  "requestIdleCallback",
  "screen",
  "self",
  "sessionStorage",
  "setInterval",
  "setTimeout",
  "skipWaiting",
  "structuredClone",
  "top",
  "undefined",
  "unescape",
  "window",
]);

function Scope(parent, kind) {
  this.parent = parent;
  this.kind = kind;
  this.names = new Set();
  this.strict = parent ? parent.strict : false;
  this.arrow = false;
}

Scope.prototype.has = function has(name) {
  if (this.names.has(name)) return true;
  return this.parent ? this.parent.has(name) : false;
};

Scope.prototype.functionScope = function functionScope() {
  let scope = this;
  while (scope && scope.kind !== "function" && scope.kind !== "script") {
    scope = scope.parent;
  }
  return scope || this;
};

Scope.prototype.hasArguments = function hasArguments() {
  let scope = this;
  while (scope) {
    if (scope.kind === "function" && !scope.arrow) return true;
    scope = scope.parent;
  }
  return false;
};

function isFunction(node) {
  return (
    node.type === "FunctionDeclaration" ||
    node.type === "FunctionExpression" ||
    node.type === "ArrowFunctionExpression"
  );
}

function hasUseStrict(body) {
  if (!body || body.type !== "BlockStatement") return false;
  return body.body.some(
    (stmt) =>
      stmt.type === "ExpressionStatement" && stmt.directive === "use strict",
  );
}

function childKeys(node) {
  const keys = [];
  for (const key of Object.keys(node)) {
    if (key === "type" || key === "start" || key === "end" || key === "loc") {
      continue;
    }
    const value = node[key];
    if (Array.isArray(value) || (value && value.type)) keys.push(key);
  }
  return keys;
}

function findUndeclared(code, filename) {
  let ast;
  try {
    ast = acorn.parse(code, {
      ecmaVersion: "latest",
      sourceType: "script",
      locations: true,
      allowReturnOutsideFunction: true,
    });
  } catch (err) {
    return [
      {
        file: filename,
        line: err.loc ? err.loc.line : 1,
        name: "<syntax>",
        message: err.message,
      },
    ];
  }

  const bindingIds = new Set();
  const scopeOf = new WeakMap();
  const script = new Scope(null, "script");

  function bind(scope, name, idNode) {
    scope.names.add(name);
    if (idNode) bindingIds.add(idNode);
  }

  function tag(node, scope) {
    if (node && node.type) scopeOf.set(node, scope);
  }

  // Binding pattern (destruktur, rest, default). Ekspresi di sisi kanan
  // default diindeks terpisah oleh pemanggil — itu referensi, bukan binding.
  function collectPattern(node, scope) {
    if (!node) return;
    tag(node, scope);
    if (node.type === "Identifier") {
      bind(scope, node.name, node);
      return;
    }
    if (node.type === "AssignmentPattern") {
      collectPattern(node.left, scope);
      return;
    }
    if (node.type === "RestElement") {
      collectPattern(node.argument, scope);
      return;
    }
    if (node.type === "ArrayPattern") {
      for (const el of node.elements) collectPattern(el, scope);
      return;
    }
    if (node.type === "ObjectPattern") {
      for (const prop of node.properties) {
        tag(prop, scope);
        if (prop.type === "RestElement") {
          collectPattern(prop, scope);
          continue;
        }
        // Kunci shorthand `{a}` bukan referensi — nama yang diikat ada di value.
        if (!prop.computed && prop.key && prop.key.type === "Identifier") {
          bindingIds.add(prop.key);
          tag(prop.key, scope);
        } else if (prop.computed) {
          index(prop.key, scope);
        }
        collectPattern(prop.value, scope);
      }
    }
  }

  function indexDefaults(pattern, scope) {
    if (!pattern) return;
    if (pattern.type === "AssignmentPattern") {
      index(pattern.right, scope);
      indexDefaults(pattern.left, scope);
      return;
    }
    if (pattern.type === "ArrayPattern") {
      for (const el of pattern.elements) indexDefaults(el, scope);
      return;
    }
    if (pattern.type === "ObjectPattern") {
      for (const prop of pattern.properties) {
        if (prop.type === "RestElement") indexDefaults(prop.argument, scope);
        else indexDefaults(prop.value, scope);
      }
    }
  }

  function bindVarDeep(node, fnScope) {
    if (!node || !node.type || isFunction(node)) return;
    if (node.type === "VariableDeclaration" && node.kind === "var") {
      for (const decl of node.declarations) collectPattern(decl.id, fnScope);
    }
    for (const key of childKeys(node)) {
      const value = node[key];
      if (Array.isArray(value)) {
        for (const child of value) bindVarDeep(child, fnScope);
      } else {
        bindVarDeep(value, fnScope);
      }
    }
  }

  function indexBlock(block, parentScope, isFunctionBody) {
    const scope = isFunctionBody
      ? new Scope(parentScope, "block")
      : new Scope(parentScope, "block");
    scope.strict = parentScope.strict || hasUseStrict(block);
    if (isFunctionBody) scope.strict = parentScope.strict || scope.strict;
    tag(block, scope);
    const fnScope = scope.functionScope();

    for (const stmt of block.body) {
      if (stmt.type === "FunctionDeclaration" && stmt.id) {
        // Di badan fungsi, deklarasi fungsi di-hoist ke scope fungsi (strict
        // maupun tidak). Di blok bersarang, strict = block-scoped.
        const target = isFunctionBody || !scope.strict ? fnScope : scope;
        bind(target, stmt.id.name, stmt.id);
      } else if (stmt.type === "ClassDeclaration" && stmt.id) {
        bind(scope, stmt.id.name, stmt.id);
      } else if (stmt.type === "VariableDeclaration" && stmt.kind !== "var") {
        for (const decl of stmt.declarations) collectPattern(decl.id, scope);
      }
    }
    if (isFunctionBody) bindVarDeep(block, fnScope);

    for (const stmt of block.body) index(stmt, scope);
  }

  function indexFunction(node, outer) {
    tag(node, outer);
    const fnScope = new Scope(outer, "function");
    fnScope.arrow = node.type === "ArrowFunctionExpression";
    fnScope.strict =
      outer.strict ||
      hasUseStrict(node.body) ||
      // Method/class selalu strict; pemanggil menandai lewat outer.strict
      // untuk class body. Arrow mewarisi.
      false;
    if (node.type !== "ArrowFunctionExpression" && outer.strict) {
      fnScope.strict = true;
    }
    if (hasUseStrict(node.body)) fnScope.strict = true;
    if (node.id && node.type !== "FunctionDeclaration") {
      bind(fnScope, node.id.name, node.id);
    } else if (node.id) {
      tag(node.id, outer);
      bindingIds.add(node.id);
    }
    for (const param of node.params) {
      collectPattern(param, fnScope);
      indexDefaults(param, fnScope);
    }
    if (node.body.type === "BlockStatement") {
      // Badan fungsi adalah block scope anak (let di badan tidak terlihat
      // dari default parameter).
      const body = new Scope(fnScope, "block");
      body.strict = fnScope.strict;
      // indexBlock membuat scope baru; kita ingin strict-nya fnScope.
      // Pakai indexBlock dengan parent = fnScope dan isFunctionBody.
      indexBlock(node.body, fnScope, true);
      // indexBlock mengabaikan strict yang kita set di `body` di atas —
      // ia membuat scope sendiri. Turunkan strict lewat parent.
      fnScope.strict = fnScope.strict;
    } else {
      index(node.body, fnScope);
    }
  }

  function index(node, scope) {
    if (!node || !node.type) return;
    tag(node, scope);

    if (isFunction(node)) {
      indexFunction(node, scope);
      return;
    }

    if (node.type === "BlockStatement") {
      indexBlock(node, scope, false);
      return;
    }

    if (node.type === "ClassDeclaration" || node.type === "ClassExpression") {
      const classScope = new Scope(scope, "block");
      classScope.strict = true;
      if (node.id && node.type === "ClassExpression") {
        bind(classScope, node.id.name, node.id);
      } else if (node.id) {
        bindingIds.add(node.id);
        tag(node.id, scope);
      }
      if (node.superClass) index(node.superClass, scope);
      tag(node.body, classScope);
      for (const item of node.body.body) {
        tag(item, classScope);
        if (item.computed) index(item.key, classScope);
        else if (item.key && item.key.type === "Identifier") {
          bindingIds.add(item.key);
          tag(item.key, classScope);
        }
        if (
          item.type === "MethodDefinition" ||
          item.type === "PropertyDefinition"
        ) {
          if (item.value) {
            // Method selalu strict.
            const saved = classScope.strict;
            classScope.strict = true;
            index(item.value, classScope);
            classScope.strict = saved;
          }
        }
      }
      return;
    }

    if (node.type === "CatchClause") {
      const catchScope = new Scope(scope, "block");
      catchScope.strict = scope.strict;
      if (node.param) collectPattern(node.param, catchScope);
      index(node.body, catchScope);
      return;
    }

    if (
      node.type === "ForStatement" ||
      node.type === "ForInStatement" ||
      node.type === "ForOfStatement"
    ) {
      const loopScope = new Scope(scope, "block");
      loopScope.strict = scope.strict;
      tag(node, loopScope);
      const head = node.init || node.left;
      if (head && head.type === "VariableDeclaration") {
        const target = head.kind === "var" ? scope.functionScope() : loopScope;
        for (const decl of head.declarations) {
          collectPattern(decl.id, target);
          if (decl.init) index(decl.init, loopScope);
        }
        tag(head, loopScope);
      } else if (head) {
        index(head, loopScope);
      }
      if (node.test) index(node.test, loopScope);
      if (node.update) index(node.update, loopScope);
      if (node.right) index(node.right, loopScope);
      index(node.body, loopScope);
      return;
    }

    if (node.type === "SwitchStatement") {
      index(node.discriminant, scope);
      const switchScope = new Scope(scope, "block");
      switchScope.strict = scope.strict;
      const fnScope = switchScope.functionScope();
      for (const item of node.cases) {
        for (const stmt of item.consequent) {
          if (stmt.type === "VariableDeclaration" && stmt.kind !== "var") {
            for (const decl of stmt.declarations) {
              collectPattern(decl.id, switchScope);
            }
          } else if (stmt.type === "FunctionDeclaration" && stmt.id) {
            const target = switchScope.strict ? switchScope : fnScope;
            bind(target, stmt.id.name, stmt.id);
          }
        }
      }
      for (const item of node.cases) {
        tag(item, switchScope);
        if (item.test) index(item.test, switchScope);
        for (const stmt of item.consequent) index(stmt, switchScope);
      }
      return;
    }

    if (node.type === "VariableDeclaration") {
      for (const decl of node.declarations) {
        tag(decl, scope);
        // id sudah diikat oleh indexBlock / for / switch. Ulangi agar
        // deklarasi di tingkat script (bukan di dalam indexBlock) ikut terikat.
        if (!scopeOf.has(decl.id)) {
          const target = node.kind === "var" ? scope.functionScope() : scope;
          collectPattern(decl.id, target);
        } else if (node.kind !== "var") {
          collectPattern(decl.id, scope);
        }
        if (decl.init) index(decl.init, scope);
      }
      return;
    }

    for (const key of childKeys(node)) {
      const value = node[key];
      if (Array.isArray(value)) {
        for (const child of value) index(child, scope);
      } else {
        index(value, scope);
      }
    }
  }

  // Script top-level: perlakukan seperti badan fungsi non-strict supaya
  // `var`/`function` di-hoist ke scope skrip.
  script.strict = hasUseStrict(ast);
  tag(ast, script);
  for (const stmt of ast.body) {
    if (stmt.type === "FunctionDeclaration" && stmt.id) {
      bind(script, stmt.id.name, stmt.id);
    } else if (stmt.type === "ClassDeclaration" && stmt.id) {
      bind(script, stmt.id.name, stmt.id);
    } else if (stmt.type === "VariableDeclaration" && stmt.kind !== "var") {
      for (const decl of stmt.declarations) collectPattern(decl.id, script);
    }
  }
  bindVarDeep(ast, script);
  for (const stmt of ast.body) index(stmt, script);

  const issues = [];
  const seen = new Set();

  function isReference(node, parent, key) {
    if (!parent) return true;
    if (bindingIds.has(node)) return false;
    if (
      parent.type === "MemberExpression" &&
      key === "property" &&
      !parent.computed
    ) {
      return false;
    }
    if (parent.type === "Property" && key === "key" && !parent.computed) {
      return false;
    }
    if (
      (parent.type === "MethodDefinition" ||
        parent.type === "PropertyDefinition") &&
      key === "key" &&
      !parent.computed
    ) {
      return false;
    }
    if (parent.type === "LabeledStatement" && key === "label") return false;
    if (
      (parent.type === "BreakStatement" ||
        parent.type === "ContinueStatement") &&
      key === "label"
    ) {
      return false;
    }
    if (parent.type === "MetaProperty") return false;
    if (
      parent.type === "ExportSpecifier" &&
      (key === "exported" || key === "local")
    ) {
      return key === "local";
    }
    return true;
  }

  function resolve(node, parent, key) {
    if (!node || !node.type) return;
    if (node.type === "Identifier" && isReference(node, parent, key)) {
      const scope = scopeOf.get(node) || scopeOf.get(parent) || script;
      const name = node.name;
      const ok =
        GLOBALS.has(name) ||
        (name === "arguments" && scope.hasArguments()) ||
        scope.has(name);
      if (!ok) {
        const line = node.loc ? node.loc.line : 1;
        const id = `${filename}:${line}:${name}`;
        if (!seen.has(id)) {
          seen.add(id);
          issues.push({ file: filename, line, name });
        }
      }
    }
    for (const childKey of childKeys(node)) {
      const value = node[childKey];
      if (Array.isArray(value)) {
        for (const child of value) resolve(child, node, childKey);
      } else {
        resolve(value, node, childKey);
      }
    }
  }

  resolve(ast, null, null);
  return issues;
}

function selfTest() {
  const cases = [
    {
      name: "videoMenu tak terdeklarasi (regresi v3.2.2)",
      code: "async function f(info) { if (videoMenu) return info.menuItemId; }",
      expect: ["videoMenu"],
    },
    {
      name: "videoMenu yang dideklarasikan bukan error",
      code: "const videoMenu = true; async function f() { return videoMenu; }",
      expect: [],
    },
    {
      name: "global chrome bukan error",
      code: "chrome.runtime.sendNativeMessage('h', {}, () => {});",
      expect: [],
    },
    {
      name: "let di luar bloknya",
      code: "function f() { if (true) { let y = 1; } return y; }",
      expect: ["y"],
    },
    {
      name: "var di-hoist ke fungsi",
      code: "function f() { if (true) { var y = 1; } return y; }",
      expect: [],
    },
    {
      name: "shorthand objek adalah referensi",
      code: "({foo});",
      expect: ["foo"],
    },
    {
      name: "destruktur tidak menandai binding sebagai referensi",
      code: "const {a} = b;",
      expect: ["b"],
    },
    {
      name: "arguments hanya di fungsi non-arrow",
      code: "function f() { return arguments.length; } const g = () => arguments;",
      expect: ["arguments"],
    },
    {
      name: "label bukan referensi",
      code: "foo: while (false) break foo;",
      expect: [],
    },
  ];
  const failures = [];
  for (const item of cases) {
    const got = findUndeclared(item.code, "<self-test>")
      .map((issue) => issue.name)
      .sort();
    const expect = [...item.expect].sort();
    if (got.join(",") !== expect.join(",")) {
      failures.push(
        `${item.name}: dapat [${got.join(", ")}], harap [${expect.join(", ")}]`,
      );
    }
  }
  if (failures.length) {
    console.error("self-test check-undeclared gagal:");
    for (const failure of failures) console.error("  - " + failure);
    process.exit(1);
  }
}

function main() {
  selfTest();
  const all = [];
  for (const rel of FILES) {
    const path = join(ROOT, rel);
    if (!existsSync(path)) {
      console.error(`berkas extension tidak ditemukan: ${rel}`);
      process.exit(1);
    }
    const issues = findUndeclared(readFileSync(path, "utf8"), rel);
    all.push(...issues);
  }
  if (all.length) {
    console.error(
      `identifier tak terdeklarasi (${all.length}) — ini kelas bug yang mematikan context menu di v3.2.2:`,
    );
    for (const issue of all) {
      console.error(`  ${issue.file}:${issue.line}  ${issue.name}`);
    }
    process.exit(1);
  }
  console.log(
    "extension: tidak ada identifier tak terdeklarasi (" +
      FILES.length +
      " berkas)",
  );
}

main();
