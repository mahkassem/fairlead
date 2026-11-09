// A download that fails on the network or with a server error is tried
// again, one that 404s isn't, a body cut short is a failure and not a
// broken archive, and FAIRLEAD_BINARY is run in place of any download.
// Run with `node --test 'npm/*.test.js'`; `INSTALLER` and `BINARY` point it at other copies.
const { test } = require("node:test");
const assert = require("node:assert");
const http = require("node:http");
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const INSTALLER = process.env.INSTALLER || path.join(__dirname, "binary-install.js");
const BINARY = process.env.BINARY || path.join(__dirname, "binary.js");
const unix = { skip: process.platform === "win32" };

function release() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-release-"));
  const bin = path.join(dir, "fairlead-x", "fairlead");
  fs.mkdirSync(path.dirname(bin));
  fs.writeFileSync(bin, "#!/bin/sh\necho ok\n");
  fs.chmodSync(bin, 0o755);
  const tarball = path.join(dir, "fairlead.tar.gz");
  assert.strictEqual(spawnSync("tar", ["czf", tarball, "-C", dir, "fairlead-x"]).status, 0);
  return fs.readFileSync(tarball);
}

const clean = (extra = {}) => {
  const env = { ...process.env, ...extra };
  for (const key of ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]) delete env[key];
  return env;
};

function node(script, env) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, ["-e", script], { env });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => resolve({ code, out: out.trim() }));
  });
}

// Serves `answers` in turn, one per request, then the release.
async function serve(answers) {
  const body = release();
  let requests = 0;
  const server = http.createServer((_, res) => {
    const answer = answers[requests++];
    if (answer === "short") {
      res.writeHead(200, { "content-length": body.length });
      res.write(body.subarray(0, 10));
      // Late enough that the headers and the first bytes have arrived.
      return setTimeout(() => res.socket.destroy(), 50);
    }
    if (answer) {
      res.writeHead(answer);
      return res.end();
    }
    res.writeHead(200, { "content-length": body.length });
    res.end(body);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${server.address().port}/fairlead.tar.gz`;
  return { url, server, requests: () => requests };
}

function firstRun(url) {
  const pkg = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-pkg-"));
  fs.copyFileSync(INSTALLER, path.join(pkg, "binary-install.js"));
  const script = `const p = new (require(${JSON.stringify(path.join(pkg, "binary-install.js"))}).Package)(
    { artifactName: "fairlead-x.tar.gz" }, "fairlead", ${JSON.stringify(url)},
    "fairlead.tar.gz", ".tar.gz", { fairlead: "fairlead" });
    p.retryDelays = [10, 10, 10];
    p.run("fairlead")`;
  return node(script, clean());
}

test("server errors and a cut connection are tried again", unix, async () => {
  const s = await serve([503, "short", 502]);
  try {
    const run = await firstRun(s.url);
    assert.strictEqual(run.code, 0, run.out);
    assert.match(run.out, /HTTP 503 .*; trying again in 0.01 s/);
    assert.match(run.out, /closed after \d+ bytes/);
    assert.match(run.out, /ok$/);
    assert.strictEqual(s.requests(), 4);
  } finally {
    s.server.close();
  }
});

test("a 404 isn't tried again", unix, async () => {
  const s = await serve([404]);
  try {
    const run = await firstRun(s.url);
    assert.notStrictEqual(run.code, 0);
    assert.match(run.out, /HTTP 404/);
    assert.strictEqual(s.requests(), 1);
  } finally {
    s.server.close();
  }
});

test("the retries end after three", unix, async () => {
  const s = await serve([500, 500, 500, 500, 500]);
  try {
    const run = await firstRun(s.url);
    assert.notStrictEqual(run.code, 0);
    assert.strictEqual(s.requests(), 4);
  } finally {
    s.server.close();
  }
});

test("which failures are worth another try", () => {
  const { transient } = require(INSTALLER);
  const status = (n) => Object.assign(new Error(), { status: n });
  const code = (c) => Object.assign(new Error(), { code: c });
  for (const e of [status(500), status(503), status(429), status(408), code("ECONNRESET"), code("ETIMEDOUT"), code("EAI_AGAIN")]) {
    assert.ok(transient(e), `${e.status || e.code}`);
  }
  for (const e of [status(404), status(403), code("EACCES"), new Error("Too many redirects")]) {
    assert.ok(!transient(e), `${e.status || e.code || e.message}`);
  }
});

test("FAIRLEAD_BINARY runs in place of a download, with the arguments", unix, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-given-"));
  const bin = path.join(dir, "fairlead");
  fs.writeFileSync(bin, '#!/bin/sh\necho "given $*"\nexit 3\n');
  fs.chmodSync(bin, 0o755);
  // No package.json beside the wrapper: nothing may read the manifest.
  const pkg = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-pkg-"));
  fs.copyFileSync(BINARY, path.join(pkg, "binary.js"));
  fs.copyFileSync(INSTALLER, path.join(pkg, "binary-install.js"));
  const wrapper = JSON.stringify(path.join(pkg, "binary.js"));
  const ran = await node(
    `process.argv = ["node", "fairlead", "doctor", "--score"]; require(${wrapper}).run("fairlead")`,
    clean({ FAIRLEAD_BINARY: bin }),
  );
  assert.deepStrictEqual(ran, { code: 3, out: "given doctor --score" });
  const installed = await node(`require(${wrapper}).install(false)`, clean({ FAIRLEAD_BINARY: bin }));
  assert.strictEqual(installed.code, 0, installed.out);
  assert.match(installed.out, /using FAIRLEAD_BINARY=.*; nothing downloaded/);
  const missing = await node(
    `require(${wrapper}).run("fairlead")`,
    clean({ FAIRLEAD_BINARY: path.join(dir, "absent") }),
  );
  assert.strictEqual(missing.code, 1);
  assert.match(missing.out, /FAIRLEAD_BINARY is .*absent, which isn't a file/);
});
