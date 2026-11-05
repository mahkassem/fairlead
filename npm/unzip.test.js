// The Windows install unzips with PowerShell, which reads every argument after
// -Command as command text: a path with a space in it must not reach it there.
// Run with `node --test 'npm/*.test.js'`; `INSTALLER` points it at another copy.
const { test } = require("node:test");
const assert = require("node:assert");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const INSTALLER = process.env.INSTALLER || path.join(__dirname, "binary-install.js");
const { expandArchive } = require(INSTALLER);

const ODD = "a user's [folder] $x";

test("the paths go in the environment, not the command", () => {
  const zip = path.join(os.tmpdir(), ODD, "fairlead.zip");
  const dest = path.join(os.tmpdir(), ODD, "out");
  const { file, args, options } = expandArchive(zip, dest);
  assert.strictEqual(file, "powershell.exe");
  assert.strictEqual(args[args.length - 2], "-Command");
  for (const arg of args) {
    assert.ok(!arg.includes(ODD), `a path reached the command line: ${arg}`);
  }
  assert.strictEqual(options.env.FAIRLEAD_ZIP, zip);
  assert.strictEqual(options.env.FAIRLEAD_DEST, dest);
});

test("PowerShell unzips into a folder with a space", { skip: process.platform !== "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "fairlead unzip "));
  try {
    const src = path.join(root, "release x", "fairlead-x");
    fs.mkdirSync(src, { recursive: true });
    fs.writeFileSync(path.join(src, "fairlead.exe"), "ok");
    const zip = path.join(root, "release x", "fairlead.zip");
    const pack = spawnSync("powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "Compress-Archive -LiteralPath $env:SRC -DestinationPath $env:ZIP",
    ], { env: { ...process.env, SRC: src, ZIP: zip } });
    assert.strictEqual(pack.status, 0, String(pack.stderr));

    const dest = path.join(root, "Some User", "node_modules", ".bin_real-x");
    fs.mkdirSync(dest, { recursive: true });
    const { file, args, options } = expandArchive(zip, dest);
    const result = spawnSync(file, args, options);
    assert.strictEqual(result.status, 0, String(result.stderr));
    const out = fs.readFileSync(path.join(dest, "fairlead-x", "fairlead.exe"), "utf8");
    assert.strictEqual(out, "ok");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
