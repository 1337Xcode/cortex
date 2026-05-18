#!/usr/bin/env node

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const platform = process.platform;
const binaryName = platform === "win32" ? "cortex.exe" : "cortex";

// Check vendor directory (bundled binary)
const vendorPath = path.join(__dirname, "..", "vendor", binaryName);
// Check bin directory (downloaded by install script)
const binPath = path.join(__dirname, binaryName);

const binaryPath = fs.existsSync(vendorPath) ? vendorPath : fs.existsSync(binPath) ? binPath : null;

if (binaryPath) {
  try {
    execFileSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });
  } catch (e) {
    process.exit(e.status || 1);
  }
} else {
  console.error("cortex binary not found.");
  console.error("");
  console.error("Looked in:");
  console.error("  " + vendorPath);
  console.error("  " + binPath);
  console.error("");
  console.error("Try: npm rebuild @1337xcode/cortex");
  console.error("Or build from source: git clone https://github.com/1337Xcode/cortex && cd cortex && cargo build --release");
  process.exit(1);
}
