#!/usr/bin/env node

/**
 * Cortex installer script.
 *
 * Downloads the correct platform binary from GitHub releases, places it in
 * ~/.cortex/bin/, and runs `cortex install` to configure detected AI agents.
 *
 * Handles fresh installs and updates: overwrites any existing binary in place.
 *
 * Usage: npx @1337xcode/cortex install
 */

const { execFileSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const https = require("node:https");

const REPO = "1337Xcode/cortex";
const INSTALL_DIR = path.join(os.homedir(), ".cortex", "bin");

function getPlatformTarget() {
  const platform = process.platform;
  const arch = process.arch;

  const supported = {
    "darwin-x64": "cortex-darwin-x64.tar.gz",
    "darwin-arm64": "cortex-darwin-arm64.tar.gz",
    "linux-x64": "cortex-linux-x64.tar.gz",
    "win32-x64": "cortex-win32-x64.tar.gz",
  };

  const key = `${platform}-${arch}`;
  const archive = supported[key];
  if (!archive) {
    throw new Error(
      `Unsupported platform: ${key}\n` +
      `Supported: ${Object.keys(supported).join(", ")}`
    );
  }
  return { platform, arch, archive };
}

function download(url) {
  return new Promise((resolve, reject) => {
    const get = (targetUrl) => {
      https.get(targetUrl, { headers: { "User-Agent": "cortex-installer" } }, (res) => {
        if (res.statusCode === 302 || res.statusCode === 301) {
          return get(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Download failed (HTTP ${res.statusCode}): ${targetUrl}`));
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      }).on("error", reject);
    };
    get(url);
  });
}

function verifySha256(buffer, expectedHash, archiveName) {
  const actualHash = crypto.createHash("sha256").update(buffer).digest("hex");
  if (actualHash !== expectedHash) {
    throw new Error(
      `SHA256 checksum mismatch for ${archiveName}!\n` +
      `  Expected: ${expectedHash}\n` +
      `  Actual:   ${actualHash}\n` +
      `The downloaded binary may have been tampered with. Aborting.`
    );
  }
  return actualHash;
}

async function downloadChecksum(archiveName, baseUrl) {
  const checksumUrl = `${baseUrl}/${archiveName}.sha256`;
  try {
    const buf = await download(checksumUrl);
    const content = buf.toString("utf8").trim();
    const hash = content.split(/\s+/)[0].toLowerCase();
    if (!hash || hash.length !== 64) {
      return null;
    }
    return hash;
  } catch {
    return null;
  }
}

async function extractTarGz(buffer, destDir) {
  const tmpFile = path.join(destDir, "_cortex_download.tar.gz");
  fs.writeFileSync(tmpFile, buffer);
  try {
    execFileSync("tar", ["-xzf", tmpFile, "-C", destDir], { stdio: "ignore" });
  } catch (e) {
    throw new Error(
      `Failed to extract archive. Ensure tar is available.\n${e.message}`
    );
  } finally {
    try { fs.unlinkSync(tmpFile); } catch { /* best-effort cleanup */ }
  }
}

function runCortexInstall(binaryPath) {
  console.log("\nConfiguring AI agents...");
  try {
    execFileSync(binaryPath, ["install"], { stdio: "inherit" });
  } catch (e) {
    // Non-fatal: binary is installed even if agent config fails
    console.warn("Warning: `cortex install` exited with an error.");
    console.warn("You can run it manually later: cortex install");
  }
}

async function main() {
  const { platform, arch, archive } = getPlatformTarget();
  const binaryName = platform === "win32" ? "cortex.exe" : "cortex";
  const baseUrl = `https://github.com/${REPO}/releases/latest/download`;
  const url = `${baseUrl}/${archive}`;

  console.log(`Installing cortex for ${platform}-${arch}...`);

  // Create install directory (handles fresh installs)
  fs.mkdirSync(INSTALL_DIR, { recursive: true });

  // Remove existing binary if present (handles updates)
  const binaryPath = path.join(INSTALL_DIR, binaryName);
  if (fs.existsSync(binaryPath)) {
    console.log("Existing installation detected, updating...");
    try {
      fs.unlinkSync(binaryPath);
    } catch (e) {
      throw new Error(
        `Cannot replace existing binary at ${binaryPath}: ${e.message}\n` +
        `Close any running cortex processes and try again.`
      );
    }
  }

  // Download the archive
  console.log(`Downloading from GitHub releases...`);
  const buffer = await download(url);

  // Verify checksum if available
  const expectedHash = await downloadChecksum(archive, baseUrl);
  if (expectedHash) {
    const actualHash = verifySha256(buffer, expectedHash, archive);
    console.log(`SHA256 verified: ${actualHash.slice(0, 12)}...`);
  } else {
    console.warn("Warning: Checksum file not available, skipping verification.");
  }

  // Extract
  await extractTarGz(buffer, INSTALL_DIR);

  // Make executable on Unix
  if (platform !== "win32" && fs.existsSync(binaryPath)) {
    fs.chmodSync(binaryPath, 0o755);
  }

  if (!fs.existsSync(binaryPath)) {
    throw new Error(
      `Binary not found after extraction. Expected: ${binaryPath}\n` +
      `The release archive may have a different structure.`
    );
  }

  console.log(`\nInstalled cortex to ${binaryPath}`);

  // Add PATH hint if not already on PATH
  const pathDirs = (process.env.PATH || "").split(path.delimiter);
  const onPath = pathDirs.some((dir) => {
    try {
      return fs.realpathSync(dir) === fs.realpathSync(INSTALL_DIR);
    } catch {
      return dir === INSTALL_DIR;
    }
  });

  if (!onPath) {
    console.log("");
    console.log("Add cortex to your PATH:");
    if (platform === "win32") {
      console.log(`  setx PATH "%PATH%;${INSTALL_DIR}"`);
    } else {
      console.log(`  export PATH="${INSTALL_DIR}:$PATH"`);
      console.log(`  # Add to ~/.bashrc or ~/.zshrc to persist`);
    }
  }

  // Run cortex install to configure agents
  runCortexInstall(binaryPath);

  console.log("\nDone. Run `cortex serve` to start the MCP server.");
}

main().catch((err) => {
  console.error(`\nInstallation failed: ${err.message}`);
  process.exit(1);
});
