#!/usr/bin/env node

const { execSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const https = require("node:https");

const REPO = "1337Xcode/cortex";
const VENDOR_DIR = path.join(__dirname, "..", "vendor");

// If binary already bundled in the package, skip download
const bundledBinary = path.join(VENDOR_DIR, process.platform === "win32" ? "cortex.exe" : "cortex");
if (fs.existsSync(bundledBinary)) {
  console.log(`cortex binary found at ${bundledBinary} (bundled)`);
  process.exit(0);
}

function getPlatform() {
  const platform = process.platform;
  if (platform === "darwin" || platform === "linux" || platform === "win32") {
    return platform;
  }
  throw new Error(`Unsupported platform: ${platform}`);
}

function getArch() {
  const arch = process.arch;
  if (arch === "x64" || arch === "arm64") {
    return arch;
  }
  throw new Error(`Unsupported architecture: ${arch}`);
}

function download(url) {
  return new Promise((resolve, reject) => {
    https.get(url, (res) => {
      if (res.statusCode === 302 || res.statusCode === 301) {
        return download(res.headers.location).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) {
        return reject(new Error(`Download failed with status ${res.statusCode}: ${url}`));
      }
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    }).on("error", reject);
  });
}

function verifySha256(buffer, expectedHash, archiveName) {
  const actualHash = crypto.createHash("sha256").update(buffer).digest("hex");
  if (actualHash !== expectedHash) {
    throw new Error(
      `SHA256 checksum mismatch for ${archiveName}!\n` +
      `  Expected: ${expectedHash}\n` +
      `  Actual:   ${actualHash}\n` +
      `The downloaded binary may have been tampered with. Aborting installation.`
    );
  }
  console.log(`SHA256 verified: ${actualHash}`);
}

async function downloadAndVerifyChecksum(archiveBuffer, archiveName, baseUrl) {
  const checksumUrl = `${baseUrl}/${archiveName}.sha256`;
  try {
    const checksumBuffer = await download(checksumUrl);
    const checksumContent = checksumBuffer.toString("utf8").trim();
    // Format is: "<hash>  <filename>" (two spaces between hash and filename)
    const expectedHash = checksumContent.split(/\s+/)[0].toLowerCase();
    if (!expectedHash || expectedHash.length !== 64) {
      console.warn(`Warning: Invalid checksum format in ${archiveName}.sha256, skipping verification.`);
      return;
    }
    verifySha256(archiveBuffer, expectedHash, archiveName);
  } catch (err) {
    if (err.message && err.message.includes("checksum mismatch")) {
      // Verification failed - this is a security issue, abort
      throw err;
    }
    // Could not download checksum file (e.g., older release without checksums)
    console.warn(`Warning: Could not download checksum file (${err.message}).`);
    console.warn("Skipping SHA256 verification. Binary integrity cannot be confirmed.");
  }
}

async function extractTarGz(buffer, destDir) {
  const tmpFile = path.join(destDir, "_cortex_download.tar.gz");
  fs.writeFileSync(tmpFile, buffer);

  try {
    execSync(`tar -xzf "${tmpFile}" -C "${destDir}"`, { stdio: "ignore" });
  } catch (e) {
    throw new Error(
      `Failed to extract archive. Ensure tar is available on your system.\n${e.message}`
    );
  } finally {
    try { fs.unlinkSync(tmpFile); } catch { /* cleanup best-effort */ }
  }
}

async function main() {
  const platform = getPlatform();
  const arch = getArch();

  const binaryName = platform === "win32" ? "cortex.exe" : "cortex";
  const archiveName = `cortex-${platform}-${arch}.tar.gz`;
  const baseUrl = `https://github.com/${REPO}/releases/latest/download`;
  const url = `${baseUrl}/${archiveName}`;

  console.log(`Downloading cortex for ${platform}-${arch}...`);

  // Ensure vendor directory exists
  fs.mkdirSync(VENDOR_DIR, { recursive: true });

  try {
    const buffer = await download(url);

    // Verify SHA256 checksum before extracting
    await downloadAndVerifyChecksum(buffer, archiveName, baseUrl);

    await extractTarGz(buffer, VENDOR_DIR);

    // Make binary executable on Unix
    if (platform !== "win32") {
      const binPath = path.join(VENDOR_DIR, binaryName);
      if (fs.existsSync(binPath)) {
        fs.chmodSync(binPath, 0o755);
      }
    }

    console.log(`Installed cortex to ${path.join(VENDOR_DIR, binaryName)}`);
  } catch (err) {
    if (err.message && err.message.includes("checksum mismatch")) {
      console.error(`\nSECURITY ERROR: ${err.message}`);
      console.error("Installation aborted for your safety.");
      process.exit(1);
    }
    console.warn(`Note: Could not download pre-built cortex binary: ${err.message}`);
    console.warn("The cortex command will not be available until you either:");
    console.warn("  1. Wait for release binaries at https://github.com/1337Xcode/cortex/releases");
    console.warn("  2. Build from source: git clone https://github.com/1337Xcode/cortex && cd cortex && cargo build --release");
    console.warn("");
    console.warn("This is not a fatal error. The package installed successfully.");
    // Don't exit with error code so npm install succeeds
  }
}

main();
