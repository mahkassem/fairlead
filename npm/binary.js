// dist 0.32.0's npm platform selection (MIT OR Apache-2.0), changed so a
// musl system gets the musl build and a platform with no build installs
// nothing instead of failing the install (mahkassem/fairlead#218).
// scripts/npm_patch.py puts it in the npm package.
const { Package } = require("./binary-install");
const { execFileSync } = require("child_process");
const { readdirSync, readFileSync } = require("fs");
const os = require("os");

const error = (msg) => {
  console.error(msg);
  process.exit(1);
};

// What Node, the filesystem and ldd say about the C library. Each probe
// answers null when it can't tell.
const probes = {
  lddFile: () => {
    try {
      return readFileSync("/usr/bin/ldd", "utf8");
    } catch {
      return null;
    }
  },
  report: () => {
    try {
      if (!process.report || !process.report.getReport) return null;
      process.report.excludeNetwork = true;
      return process.report.getReport();
    } catch {
      return null;
    }
  },
  lib: () => {
    try {
      return readdirSync("/lib");
    } catch {
      return null;
    }
  },
  ldd: () => {
    try {
      return execFileSync("ldd", ["--version"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch (e) {
      // musl's ldd prints its banner on stderr and exits 1.
      return e && (e.stderr || e.stdout) ? `${e.stdout || ""}${e.stderr || ""}` : null;
    }
  },
};

const lddVersion = (probe) => {
  const m = (probe.ldd() || "").split("\n")[0].match(/(\d+\.\d+)\s*$/);
  return m ? m[1] : undefined;
};

// { family: "glibc", version } or { family: "musl" }, or null when neither
// shows. The ldd script names its C library and is read first, since a
// runtime other than Node may report a glibc it doesn't run on; then Node's
// report, the musl loader in /lib, and `ldd --version`.
const detectLibc = (probe = probes) => {
  const file = probe.lddFile() || "";
  if (/musl/.test(file)) return { family: "musl" };
  const report = probe.report();
  const runtime = report && report.header && report.header.glibcVersionRuntime;
  if (/GNU C Library/.test(file)) {
    return { family: "glibc", version: runtime || lddVersion(probe) };
  }
  if (runtime) return { family: "glibc", version: runtime };
  const objects = (report && report.sharedObjects) || [];
  if (objects.some((o) => /ld-musl-|libc\.musl-/.test(o))) {
    return { family: "musl" };
  }
  const lib = probe.lib() || [];
  if (lib.some((f) => f.startsWith("ld-musl-"))) return { family: "musl" };
  const ldd = probe.ldd() || "";
  if (/musl/i.test(ldd)) return { family: "musl" };
  if (/glibc|gnu libc/i.test(ldd)) {
    return { family: "glibc", version: lddVersion(probe) };
  }
  return null;
};

const currentHost = () => ({
  type: os.type(),
  arch: os.arch(),
  libc: os.type() === "Linux" ? detectLibc() : null,
});

const glibcTooOld = (version, minimum) => {
  if (!version || !minimum) return false;
  const [major, minor] = version.split(".").map((n) => parseInt(n, 10));
  return major !== minimum.major || minor < minimum.series;
};

// The rust target key for a host, as the package's supportedPlatforms names
// it, and the build it maps to: `platform` is null when there is none.
const selectPlatform = (host, supportedPlatforms, glibcMinimum) => {
  const osType = {
    Windows_NT: "pc-windows-msvc",
    Darwin: "apple-darwin",
    Linux: "unknown-linux-gnu",
  }[host.type];
  const arch = { x64: "x86_64", arm64: "aarch64" }[host.arch];
  let suffix = osType || "";
  let warning = null;
  if (host.type === "Linux") {
    const libc = host.libc;
    if (libc && libc.family === "musl") {
      suffix = "unknown-linux-musl-dynamic";
    } else if (!libc || libc.family !== "glibc") {
      warning = "Your libc is neither glibc nor musl; trying the static musl build instead";
      suffix = "unknown-linux-musl-static";
    } else if (glibcTooOld(libc.version, glibcMinimum)) {
      warning = "Your glibc isn't compatible; trying the static musl build instead";
      suffix = "unknown-linux-musl-static";
    }
  }
  const key = `${arch || ""}-${suffix}`;
  const platform = (arch && osType && supportedPlatforms[key]) || null;
  return { key, platform, warning };
};

const describeHost = (host) => {
  const libc = host.libc ? `, ${host.libc.family}` : "";
  return `${host.type} ${host.arch}${libc}`;
};

const unsupported = (host, manifest) => {
  const { name, version, repository, supportedPlatforms } = manifest;
  const raw = typeof repository === "string" ? repository : repository && repository.url;
  const repo = raw && raw.replace(/^git\+/, "").replace(/\.git$/, "");
  const build = repo
    ? ["Build it from source with Rust, then run that binary instead:",
       `  cargo install --locked --git ${repo} --tag v${version} ${name}`]
    : ["Build it from source with Rust, then run that binary instead."];
  return [
    `${name} isn't installed for this platform (${describeHost(host)}): no release build runs on it.`,
    ...build,
    `Release builds exist for: ${Object.keys(supportedPlatforms).join(", ")}`,
  ].join("\n");
};

const manifest = () => require("./package.json");

const getPackage = (host = currentHost()) => {
  const pkg = manifest();
  const chosen = selectPlatform(host, pkg.supportedPlatforms, pkg.glibcMinimum);
  if (!chosen.platform) return { missing: unsupported(host, pkg) };
  if (chosen.warning) console.warn(chosen.warning);
  // FIXME: implement NPM installer handling of fallback download URLs
  const url = `${pkg.artifactDownloadUrls[0]}/${chosen.platform.artifactName}`;
  const { artifactName, zipExt, bins } = chosen.platform;
  return new Package(chosen.platform, pkg.name, url, artifactName, zipExt, bins);
};

// The postinstall step. A platform with no build warns and succeeds, so a
// project that never runs the tool still installs; running it says why.
const install = (suppressLogs, host) => {
  const urls = manifest().artifactDownloadUrls;
  if (!urls || !urls[0] || urls[0].length === 0) {
    console.warn("in demo mode, not installing binaries");
    return;
  }
  const pkg = getPackage(host);
  if (pkg.missing) {
    console.warn(`warning: ${pkg.missing}`);
    return;
  }
  return pkg.install(suppressLogs);
};

const run = (binaryName, host) => {
  const pkg = getPackage(host);
  if (pkg.missing) error(pkg.missing);
  pkg.run(binaryName);
};

module.exports = {
  install,
  run,
  getPackage,
  detectLibc,
  selectPlatform,
  unsupported,
};
