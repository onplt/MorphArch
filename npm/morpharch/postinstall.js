// Fallback check: verify that the platform binary was installed
const os = require("os");
const path = require("path");

const PLATFORM_MAP = {
  "linux-x64": "@morpharch/cli-linux-x64",
  "linux-arm64": "@morpharch/cli-linux-arm64",
  "darwin-x64": "@morpharch/cli-darwin-x64",
  "darwin-arm64": "@morpharch/cli-darwin-arm64",
  "win32-x64": "@morpharch/cli-win32-x64",
};

const key = `${os.platform()}-${os.arch()}`;
const pkg = PLATFORM_MAP[key];

if (!pkg) {
  console.warn(
    `[morpharch] Warning: No prebuilt binary for ${key}.\n` +
      `You can install from source: cargo install morpharch`
  );
  process.exit(0);
}

try {
  require.resolve(`${pkg}/package.json`);
} catch {
  console.warn(
    `[morpharch] Warning: Platform package ${pkg} was not installed.\n` +
      `This can happen if npm skipped optional dependencies.\n` +
      `Try: npm install -g morpharch --include=optional\n` +
      `Or install from source: cargo install morpharch`
  );
}
