#!/usr/bin/env node

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");
const os = require("os");

const platform = process.platform;
const binaryName = platform === "win32" ? "cortex.exe" : "cortex";

// Binary search locations (in priority order)
const searchPaths = [
  // 1. Persistent install location (~/.cortex/bin/)
  path.join(os.homedir(), ".cortex", "bin", binaryName),
  // 2. Vendor directory (bundled in package)
  path.join(__dirname, "..", "vendor", binaryName),
  // 3. Local bin directory (legacy)
  path.join(__dirname, binaryName),
];

const binaryPath = searchPaths.find((p) => fs.existsSync(p));
const args = process.argv.slice(2);

// If the user runs `npx @1337xcode/cortex install` and no binary exists yet,
// run the install script directly to bootstrap the binary.
if (!binaryPath && args[0] === "install") {
  const installScript = path.join(__dirname, "..", "scripts", "install.js");
  try {
    execFileSync(process.execPath, [installScript], { stdio: "inherit" });
  } catch (e) {
    process.exit(e.status || 1);
  }
  process.exit(0);
}

if (binaryPath) {
  // If the command is "install" and the binary exists, we still run the install
  // script to handle updates (re-download latest binary + reconfigure agents).
  if (args[0] === "install") {
    const installScript = path.join(__dirname, "..", "scripts", "install.js");
    try {
      execFileSync(process.execPath, [installScript], { stdio: "inherit" });
    } catch (e) {
      process.exit(e.status || 1);
    }
    process.exit(0);
  }

  try {
    execFileSync(binaryPath, args, { stdio: "inherit" });
  } catch (e) {
    process.exit(e.status || 1);
  }
} else {
  console.error("cortex binary not found.");
  console.error("");
  console.error("Run the installer:");
  console.error("  npx @1337xcode/cortex install");
  console.error("");
  console.error("Or build from source:");
  console.error("  git clone https://github.com/1337Xcode/cortex && cd cortex && cargo build --release");
  process.exit(1);
}
