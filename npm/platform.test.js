// The npm package picks the build a host can run: glibc's on glibc, the musl
// build on musl and wherever glibc is too old, and none on a platform with no
// build, where the install still succeeds and running the tool says why (#218).
// Run with `node --test 'npm/*.test.js'`; `BINARY` points it at another copy.
const { test } = require("node:test");
const assert = require("node:assert");
const http = require("node:http");
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const BINARY = process.env.BINARY || path.join(__dirname, "binary.js");
const { detectLibc, selectPlatform, unsupported } = require(BINARY);

const build = (triple) => ({
  artifactName: `fairlead-${triple}.tar.gz`,
  bins: { fairlead: "fairlead" },
  zipExt: ".tar.gz",
});

// The keys dist writes for a release with both linux libcs on both arches.
const SUPPORTED = {
  "aarch64-apple-darwin": build("aarch64-apple-darwin"),
  "aarch64-unknown-linux-gnu": build("aarch64-unknown-linux-gnu"),
  "aarch64-unknown-linux-musl-dynamic": build("aarch64-unknown-linux-musl"),
  "aarch64-unknown-linux-musl-static": build("aarch64-unknown-linux-musl"),
  "x86_64-pc-windows-msvc": build("x86_64-pc-windows-msvc"),
  "x86_64-unknown-linux-gnu": build("x86_64-unknown-linux-gnu"),
  "x86_64-unknown-linux-musl-dynamic": build("x86_64-unknown-linux-musl"),
  "x86_64-unknown-linux-musl-static": build("x86_64-unknown-linux-musl"),
};
const GLIBC_MIN = { major: 2, series: 31 };

const linux = (arch, libc) => ({ type: "Linux", arch, libc });
const pick = (host, supported = SUPPORTED) =>
  selectPlatform(host, supported, GLIBC_MIN);

test("glibc gets the gnu build", () => {
  for (const [arch, triple] of [["x64", "x86_64"], ["arm64", "aarch64"]]) {
    const got = pick(linux(arch, { family: "glibc", version: "2.39" }));
    assert.strictEqual(got.platform.artifactName, `fairlead-${triple}-unknown-linux-gnu.tar.gz`);
    assert.strictEqual(got.warning, null);
  }
});

test("musl gets the musl build", () => {
  for (const [arch, triple] of [["x64", "x86_64"], ["arm64", "aarch64"]]) {
    const got = pick(linux(arch, { family: "musl" }));
    assert.strictEqual(got.key, `${triple}-unknown-linux-musl-dynamic`);
    assert.strictEqual(got.platform.artifactName, `fairlead-${triple}-unknown-linux-musl.tar.gz`);
  }
});

test("an old glibc or an unknown libc falls back to the static musl build", () => {
  const old = pick(linux("x64", { family: "glibc", version: "2.28" }));
  assert.strictEqual(old.key, "x86_64-unknown-linux-musl-static");
  assert.match(old.warning, /glibc isn't compatible/);
  const unknown = pick(linux("x64", null));
  assert.strictEqual(unknown.key, "x86_64-unknown-linux-musl-static");
  assert.match(unknown.warning, /neither glibc nor musl/);
});

test("a platform with no build selects nothing", () => {
  const noMusl = Object.fromEntries(Object.entries(SUPPORTED).filter(([k]) => !k.includes("musl")));
  assert.strictEqual(pick(linux("x64", { family: "musl" }), noMusl).platform, null);
  assert.strictEqual(pick(linux("ia32", { family: "glibc", version: "2.39" })).platform, null);
  assert.strictEqual(pick({ type: "FreeBSD", arch: "x64", libc: null }).platform, null);
  assert.strictEqual(pick({ type: "Darwin", arch: "x64", libc: null }).platform, null);
});

test("the message for a platform with no build names the source build", () => {
  const host = { type: "FreeBSD", arch: "x64", libc: null };
  const base = { name: "fairlead", version: "1.2.3", supportedPlatforms: SUPPORTED };
  const line = "cargo install --locked --git https://example.invalid/fairlead --tag v1.2.3 fairlead";
  for (const repository of [
    "https://example.invalid/fairlead",
    { type: "git", url: "git+https://example.invalid/fairlead.git" },
  ]) {
    assert.ok(unsupported(host, { ...base, repository }).includes(line), JSON.stringify(repository));
  }
  assert.doesNotMatch(unsupported(host, base), /cargo install/);
});

test("libc detection reads the ldd script, Node's report, the musl loader, then ldd", () => {
  const probe = (lddFile, report, lib, ldd) => ({
    lddFile: () => lddFile,
    report: () => report,
    lib: () => lib,
    ldd: () => ldd,
  });
  const glibcReport = { header: { glibcVersionRuntime: "2.39" }, sharedObjects: [] };
  const glibcScript = "#!/bin/bash\n# This file is part of the GNU C Library.\n";
  const muslScript = "#!/bin/sh\nexec /lib/ld-musl-x86_64.so.1 --list \"$@\"\n";
  const glibcLdd = "ldd (GNU libc) 2.35\nCopyright (C) 2022\n";
  const muslLdd = "musl libc (x86_64)\nVersion 1.2.5\n";
  // Alpine's ldd script wins over a runtime that reports a glibc version.
  assert.deepStrictEqual(detectLibc(probe(muslScript, glibcReport, null, null)), { family: "musl" });
  // A glibc host with musl-tools installed has the musl loader too.
  assert.deepStrictEqual(
    detectLibc(probe(glibcScript, glibcReport, ["ld-musl-x86_64.so.1"], null)),
    { family: "glibc", version: "2.39" },
  );
  assert.deepStrictEqual(detectLibc(probe(glibcScript, null, null, glibcLdd)), { family: "glibc", version: "2.35" });
  assert.deepStrictEqual(detectLibc(probe(null, glibcReport, null, null)), { family: "glibc", version: "2.39" });
  const muslReport = { header: {}, sharedObjects: ["/lib/ld-musl-x86_64.so.1"] };
  assert.deepStrictEqual(detectLibc(probe(null, muslReport, null, null)), { family: "musl" });
  assert.deepStrictEqual(detectLibc(probe(null, null, ["ld-musl-aarch64.so.1"], null)), { family: "musl" });
  assert.deepStrictEqual(detectLibc(probe(null, null, ["libc.so"], muslLdd)), { family: "musl" });
  assert.deepStrictEqual(detectLibc(probe(null, null, [], glibcLdd)), { family: "glibc", version: "2.35" });
  assert.strictEqual(detectLibc(probe(null, null, null, null)), null);
});

test("this host's libc is detected", { skip: process.platform !== "linux" }, () => {
  assert.ok(detectLibc(), "a linux host shows glibc or musl");
});

// A package laid out as dist ships it, with this installer, whose only build
// is `triple` at `url`.
function packageDir(triple, url) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-pkg-"));
  fs.copyFileSync(BINARY, path.join(dir, "binary.js"));
  fs.copyFileSync(path.join(path.dirname(BINARY), "binary-install.js"), path.join(dir, "binary-install.js"));
  const manifest = {
    name: "fairlead",
    version: "0.0.0",
    repository: "https://example.invalid/fairlead",
    artifactDownloadUrls: [url],
    glibcMinimum: GLIBC_MIN,
    supportedPlatforms: { [triple]: { ...build(triple), artifactName: "fairlead.tar.gz" } },
  };
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(manifest));
  return dir;
}

function node(dir, call) {
  const script = `require(${JSON.stringify(path.join(dir, "binary.js"))}).${call}`;
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

const FREEBSD = JSON.stringify({ type: "FreeBSD", arch: "x64", libc: null });

test("on a platform with no build, the install warns and succeeds and the run fails", async () => {
  const dir = packageDir("x86_64-unknown-linux-gnu", "http://127.0.0.1:9/unused");
  try {
    const install = await node(dir, `install(false, ${FREEBSD})`);
    assert.strictEqual(install.code, 0, install.out);
    assert.match(install.out, /^warning: fairlead isn't installed for this platform \(FreeBSD x64\)/);
    const run = await node(dir, `run("fairlead", ${FREEBSD})`);
    assert.notStrictEqual(run.code, 0);
    assert.match(run.out, /isn't installed for this platform/);
    assert.match(run.out, /cargo install --locked --git https:\/\/example\.invalid\/fairlead --tag v0\.0\.0 fairlead/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a first run on musl downloads the musl build and runs it", { skip: process.platform === "win32" }, async () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead-release-"));
  const bin = path.join(work, "fairlead-x", "fairlead");
  fs.mkdirSync(path.dirname(bin));
  fs.writeFileSync(bin, "#!/bin/sh\necho musl-ok\n");
  fs.chmodSync(bin, 0o755);
  const tarball = path.join(work, "fairlead.tar.gz");
  assert.strictEqual(spawnSync("tar", ["czf", tarball, "-C", work, "fairlead-x"]).status, 0);
  const body = fs.readFileSync(tarball);
  const asked = [];
  const server = http.createServer((req, res) => {
    asked.push(req.url);
    res.writeHead(200, { "content-length": body.length });
    res.end(body);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const dir = packageDir("x86_64-unknown-linux-musl-dynamic", `http://127.0.0.1:${server.address().port}/v0.0.0`);
  try {
    const musl = JSON.stringify(linux("x64", { family: "musl" }));
    const run = await node(dir, `run("fairlead", ${musl})`);
    assert.deepStrictEqual(run, { code: 0, out: "musl-ok" });
    assert.deepStrictEqual(asked, ["/v0.0.0/fairlead.tar.gz"]);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
    fs.rmSync(work, { recursive: true, force: true });
  }
});
