#!/usr/bin/env node

/**
 * Cortex installer script.
 *
 * Downloads the correct platform binary from GitHub releases, places it in
 * ~/.cortex/bin/, removes stale binaries from other locations, configures PATH,
 * and runs `cortex reindex` to build the code graph.
 *
 * Handles fresh installs and updates: overwrites any existing binary in place.
 * Automatically cleans up old cortex binaries from ~/.local/bin, npm global, etc.
 *
 * Usage: npx @1337xcode/cortex install
 */

const { execFileSync, execSync } = require("node:child_process");
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
    "win32-ia32": "cortex-win32-ia32.tar.gz",
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

/**
 * Remove stale cortex binaries from known locations that are NOT our install dir.
 * This prevents old versions from shadowing the new install on PATH.
 */
function removeStaleBindaries(platform, binaryName) {
  const home = os.homedir();
  const stalePaths = [];

  if (platform === "win32") {
    // Common stale locations on Windows
    stalePaths.push(
      path.join(home, ".local", "bin", "cortex.exe"),
      path.join(home, "AppData", "Roaming", "npm", "cortex"),
      path.join(home, "AppData", "Roaming", "npm", "cortex.cmd"),
      path.join(home, "AppData", "Roaming", "npm", "cortex.ps1"),
    );
  } else {
    // Common stale locations on Unix
    stalePaths.push(
      path.join(home, ".local", "bin", "cortex"),
      "/usr/local/bin/cortex",
    );
  }

  // Also check npm global prefix
  try {
    const npmPrefix = execSync("npm prefix -g", { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }).trim();
    if (npmPrefix) {
      const npmBin = platform === "win32" ? npmPrefix : path.join(npmPrefix, "bin");
      stalePaths.push(path.join(npmBin, binaryName));
      if (platform === "win32") {
        stalePaths.push(path.join(npmPrefix, "cortex.cmd"));
        stalePaths.push(path.join(npmPrefix, "cortex.ps1"));
      }
    }
  } catch { /* npm not available or failed, skip */ }

  let removed = 0;
  for (const p of stalePaths) {
    // Never remove our own install location
    if (path.resolve(p) === path.resolve(path.join(INSTALL_DIR, binaryName))) continue;

    if (fs.existsSync(p)) {
      try {
        fs.unlinkSync(p);
        console.log(`  Removed stale binary: ${p}`);
        removed++;
      } catch {
        // Best-effort: might be locked or permission denied
        console.warn(`  Warning: Could not remove stale binary: ${p}`);
      }
    }
  }
  return removed;
}

function configurePath(installDir) {
  if (process.platform === "win32") {
    configurePathWindows(installDir);
  } else {
    configurePathUnix(installDir);
  }
}

function configurePathWindows(installDir) {
  const currentPath = process.env.PATH || "";
  if (currentPath.toLowerCase().includes(installDir.toLowerCase())) {
    return; // Already on PATH
  }
  try {
    // Prepend so new binary takes precedence
    execFileSync("setx", ["PATH", `${installDir};%PATH%`], { stdio: "ignore" });
    console.log(`Added ${installDir} to PATH (via setx).`);
  } catch (e) {
    console.warn(`Warning: Could not update PATH via setx: ${e.message}`);
    console.warn(`  Manually add to PATH: ${installDir}`);
  }
}

function configurePathUnix(installDir) {
  const exportLine = `export PATH="${installDir}:$PATH"`;
  const shellFiles = [".bashrc", ".zshrc", ".profile"]
    .map((f) => path.join(os.homedir(), f))
    .filter((f) => fs.existsSync(f));

  if (shellFiles.length === 0) {
    console.log(`Add to your shell config: ${exportLine}`);
    return;
  }

  for (const file of shellFiles) {
    const content = fs.readFileSync(file, "utf8");
    if (content.includes(installDir)) {
      continue; // Already configured
    }
    fs.appendFileSync(file, `\n# Added by cortex installer\n${exportLine}\n`);
    console.log(`Updated ${path.basename(file)} with PATH entry.`);
  }
}

function runReindex(binaryPath) {
  console.log("\nRebuilding code graph...");
  try {
    execFileSync(binaryPath, ["reindex"], { stdio: "inherit", timeout: 120000 });
  } catch {
    console.warn("Warning: `cortex reindex` failed. You can run it manually later: cortex reindex");
  }
}

async function main() {
  const { platform, arch, archive } = getPlatformTarget();
  const binaryName = platform === "win32" ? "cortex.exe" : "cortex";
  const baseUrl = `https://github.com/${REPO}/releases/latest/download`;
  const url = `${baseUrl}/${archive}`;

  console.log(`Installing cortex for ${platform}-${arch}...`);

  // Step 1: Remove stale binaries from other locations
  console.log("\nCleaning up old installations...");
  const removed = removeStaleBindaries(platform, binaryName);
  if (removed > 0) {
    console.log(`  Cleaned ${removed} stale binary location(s).`);
  } else {
    console.log("  No stale binaries found.");
  }

  // Step 2: Create install directory
  fs.mkdirSync(INSTALL_DIR, { recursive: true });

  // Step 3: Remove existing binary in our install dir (handles updates)
  const binaryPath = path.join(INSTALL_DIR, binaryName);
  if (fs.existsSync(binaryPath)) {
    console.log("\nExisting installation detected, updating...");
    try {
      fs.unlinkSync(binaryPath);
    } catch (e) {
      throw new Error(
        `Cannot replace existing binary at ${binaryPath}: ${e.message}\n` +
        `Close any running cortex processes and try again.`
      );
    }
  }

  // Step 4: Download the archive
  console.log(`\nDownloading from GitHub releases...`);
  const buffer = await download(url);

  // Step 5: Verify checksum if available
  const expectedHash = await downloadChecksum(archive, baseUrl);
  if (expectedHash) {
    const actualHash = verifySha256(buffer, expectedHash, archive);
    console.log(`SHA256 verified: ${actualHash.slice(0, 12)}...`);
  } else {
    console.warn("Warning: Checksum file not available, skipping verification.");
  }

  // Step 6: Extract
  await extractTarGz(buffer, INSTALL_DIR);

  // Step 7: Make executable on Unix
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

  // Step 8: Configure PATH (prepend so new binary always wins)
  configurePath(INSTALL_DIR);

  // Step 9: Update current session PATH so reindex uses the new binary
  process.env.PATH = `${INSTALL_DIR}${path.delimiter}${process.env.PATH}`;

  // Step 10: Run cortex reindex to build the code graph
  runReindex(binaryPath);

  console.log("\nDone. Run `cortex serve` to start the MCP server.");
  if (platform === "win32") {
    console.log("Note: Open a new terminal for PATH changes to take effect.");
  }
}

main().catch((err) => {
  console.error(`\nInstallation failed: ${err.message}`);
  process.exit(1);
});
