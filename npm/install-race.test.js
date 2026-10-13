// Several first runs at once each install the binary and run it (#211):
// none may spawn a half-written binary or lose the folder to another run.
// Run with `node --test npm/`; `INSTALLER` points it at another copy.
const { test } = require("node:test");
const assert = require("node:assert");
const http = require("node:http");
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const INSTALLER = process.env.INSTALLER || path.join(__dirname, "binary-install.js");

function release() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-release-"));
  const bin = path.join(dir, "fairlead-x", "fairlead");
  fs.mkdirSync(path.dirname(bin));
  // Padding makes extraction slow enough for runs to overlap.
  fs.writeFileSync(bin, `#!/bin/sh\necho ok\nexit 0\n#${"x".repeat(32 << 20)}\n`);
  fs.chmodSync(bin, 0o755);
  const tarball = path.join(dir, "fairlead.tar.gz");
  const tar = spawnSync("tar", ["czf", tarball, "-C", dir, "fairlead-x"]);
  assert.strictEqual(tar.status, 0, String(tar.stderr));
  return fs.readFileSync(tarball);
}

function firstRun(pkg, url) {
  const script = `new (require(${JSON.stringify(path.join(pkg, "binary-install.js"))}).Package)(
    { artifactName: "fairlead-x.tar.gz" }, "fairlead", ${JSON.stringify(url)},
    "fairlead.tar.gz", ".tar.gz", { fairlead: "fairlead" }).run("fairlead")`;
  const env = { ...process.env };
  for (const key of ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]) delete env[key];
  return new Promise((resolve) => {
    const child = spawn(process.execPath, ["-e", script], { env });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => resolve({ code, out: out.trim() }));
  });
}

test("first runs at once each get a whole binary", { skip: process.platform === "win32" }, async () => {
  const body = release();
  const server = http.createServer((_, res) => {
    res.writeHead(200, { "content-length": body.length });
    res.end(body);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${server.address().port}/fairlead.tar.gz`;
  try {
    for (let round = 0; round < 3; round++) {
      const pkg = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-pkg-"));
      fs.copyFileSync(INSTALLER, path.join(pkg, "binary-install.js"));
      const runs = await Promise.all(Array.from({ length: 8 }, () => firstRun(pkg, url)));
      const failed = runs.filter((r) => r.code !== 0 || r.out !== "ok");
      assert.deepStrictEqual(failed, [], `round ${round}`);
      const left = fs.readdirSync(path.join(pkg, "node_modules"));
      assert.deepStrictEqual(left, [".bin_real"], "no extraction left behind");
      fs.rmSync(pkg, { recursive: true, force: true });
    }
  } finally {
    server.close();
  }
});
