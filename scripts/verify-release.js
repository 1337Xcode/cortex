#!/usr/bin/env node

/**
 * Release Readiness Verification Script
 *
 * Runs three property checks to ensure the release pipeline is consistent:
 *   1. Archive naming consistency across distribution pipeline
 *   2. Documentation link integrity
 *   3. Version string consistency
 *
 * Usage: node scripts/verify-release.js
 * Exit code 0 = all pass, 1 = any fail
 */

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
let failures = 0;

function pass(msg) {
  console.log(`  ✓ PASS: ${msg}`);
}

function fail(msg) {
  console.log(`  ✗ FAIL: ${msg}`);
  failures++;
}

// ---------------------------------------------------------------------------
// Property 1: Archive naming consistency across distribution pipeline
// ---------------------------------------------------------------------------

function checkArchiveNaming() {
  console.log("\n═══ Property 1: Archive naming consistency ═══\n");

  const expectedCombinations = [
    { platform: "linux", arch: "x64" },
    { platform: "darwin", arch: "x64" },
    { platform: "darwin", arch: "arm64" },
    { platform: "win32", arch: "x64" },
  ];

  // Parse release workflow YAML for artifact names
  const workflowPath = path.resolve(ROOT, ".github", "workflows", "release.yml");
  if (!fs.existsSync(workflowPath)) {
    fail(`Release workflow not found at ${workflowPath}`);
    return;
  }

  const workflowContent = fs.readFileSync(workflowPath, "utf8");

  // Extract artifact names from the matrix (lines like "artifact: cortex-linux-x64")
  const artifactRegex = /artifact:\s*(cortex-[\w-]+)/g;
  const workflowArtifacts = new Set();
  let match;
  while ((match = artifactRegex.exec(workflowContent)) !== null) {
    workflowArtifacts.add(match[1]);
  }

  // Check install.js constructs the same pattern
  const installJsPath = path.join(ROOT, "npm", "scripts", "install.js");
  if (!fs.existsSync(installJsPath)) {
    fail(`install.js not found at ${installJsPath}`);
    return;
  }
  const installJsContent = fs.readFileSync(installJsPath, "utf8");

  // Verify install.js uses `cortex-${platform}-${arch}` pattern
  const installJsHasPattern =
    installJsContent.includes("cortex-${platform}-${arch}") ||
    installJsContent.includes("`cortex-${platform}-${arch}");
  if (!installJsHasPattern) {
    fail("install.js does not use the expected `cortex-${platform}-${arch}` naming pattern");
  }

  // Check install.sh constructs the same pattern
  const installShPath = path.join(ROOT, "install.sh");
  if (!fs.existsSync(installShPath)) {
    fail(`install.sh not found at ${installShPath}`);
    return;
  }
  const installShContent = fs.readFileSync(installShPath, "utf8");

  // Verify install.sh maps uname to the correct platform/arch values
  const shHasLinux = installShContent.includes("OS=linux");
  const shHasDarwin = installShContent.includes("OS=darwin");
  const shHasWin32 = installShContent.includes("OS=win32");
  const shHasX64 = installShContent.includes("ARCH=x64");
  const shHasArm64 = installShContent.includes("ARCH=arm64");
  const shHasPattern =
    installShContent.includes("cortex-${OS}-${ARCH}");

  if (!shHasPattern) {
    fail("install.sh does not use the expected `cortex-${OS}-${ARCH}` naming pattern");
  }
  if (!shHasLinux || !shHasDarwin || !shHasWin32) {
    fail("install.sh is missing one or more platform mappings (linux, darwin, win32)");
  }
  if (!shHasX64 || !shHasArm64) {
    fail("install.sh is missing one or more arch mappings (x64, arm64)");
  }

  // For each expected combination, verify the workflow has a matching artifact
  for (const { platform, arch } of expectedCombinations) {
    const expectedName = `cortex-${platform}-${arch}`;
    if (workflowArtifacts.has(expectedName)) {
      pass(`Workflow artifact "${expectedName}" matches install script pattern`);
    } else {
      fail(`Workflow missing artifact "${expectedName}" (found: ${[...workflowArtifacts].join(", ")})`);
    }
  }

  if (installJsHasPattern) {
    pass("install.js uses correct naming pattern: cortex-${platform}-${arch}");
  }
  if (shHasPattern && shHasLinux && shHasDarwin && shHasWin32 && shHasX64 && shHasArm64) {
    pass("install.sh uses correct naming pattern with proper uname mappings");
  }
}

// ---------------------------------------------------------------------------
// Property 2: Documentation link integrity
// ---------------------------------------------------------------------------

function checkDocLinks() {
  console.log("\n═══ Property 2: Documentation link integrity ═══\n");

  const filesToCheck = ["README.md", "CONTRIBUTING.md", "SECURITY.md"];
  const linkRegex = /\[([^\]]*)\]\(([^)]+)\)/g;

  for (const file of filesToCheck) {
    const filePath = path.join(ROOT, file);
    if (!fs.existsSync(filePath)) {
      fail(`${file} does not exist`);
      continue;
    }

    const content = fs.readFileSync(filePath, "utf8");
    let fileHasIssue = false;
    let linksChecked = 0;

    let linkMatch;
    while ((linkMatch = linkRegex.exec(content)) !== null) {
      const linkTarget = linkMatch[2];

      // Skip external URLs, anchors, and empty links
      if (
        linkTarget.startsWith("http://") ||
        linkTarget.startsWith("https://") ||
        linkTarget.startsWith("#") ||
        linkTarget === ""
      ) {
        continue;
      }

      // Strip any anchor from the path
      const cleanPath = linkTarget.split("#")[0];
      if (!cleanPath) continue;

      const resolvedPath = path.resolve(path.dirname(filePath), cleanPath);
      linksChecked++;

      if (!fs.existsSync(resolvedPath)) {
        fail(`${file}: broken link [${linkMatch[1]}](${linkTarget}) → file not found: ${cleanPath}`);
        fileHasIssue = true;
      }
    }

    if (!fileHasIssue && linksChecked > 0) {
      pass(`${file}: all ${linksChecked} relative link(s) resolve correctly`);
    } else if (linksChecked === 0) {
      pass(`${file}: no relative links to check`);
    }
  }

  // Also verify LICENSE exists (referenced by README badge)
  const licensePath = path.join(ROOT, "LICENSE");
  if (fs.existsSync(licensePath)) {
    pass("LICENSE file exists");
  } else {
    fail("LICENSE file is missing");
  }
}

// ---------------------------------------------------------------------------
// Property 3: Version string consistency
// ---------------------------------------------------------------------------

function checkVersionConsistency() {
  console.log("\n═══ Property 3: Version string consistency ═══\n");

  const versions = {};

  // Extract from Cargo.toml
  const cargoPath = path.join(ROOT, "Cargo.toml");
  if (fs.existsSync(cargoPath)) {
    const cargoContent = fs.readFileSync(cargoPath, "utf8");
    const cargoMatch = cargoContent.match(/^version\s*=\s*"([^"]+)"/m);
    if (cargoMatch) {
      versions["Cargo.toml"] = cargoMatch[1];
    } else {
      fail("Cargo.toml: could not extract version");
    }
  } else {
    fail("Cargo.toml not found");
  }

  // Extract from npm/package.json
  const pkgPath = path.join(ROOT, "npm", "package.json");
  if (fs.existsSync(pkgPath)) {
    try {
      const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
      versions["npm/package.json"] = pkg.version;
    } catch (e) {
      fail(`npm/package.json: failed to parse JSON - ${e.message}`);
    }
  } else {
    fail("npm/package.json not found");
  }

  // Extract from CHANGELOG.md (first ## [X.Y.Z] header)
  const changelogPath = path.join(ROOT, "CHANGELOG.md");
  if (fs.existsSync(changelogPath)) {
    const changelogContent = fs.readFileSync(changelogPath, "utf8");
    const changelogMatch = changelogContent.match(/^## \[([^\]]+)\]/m);
    if (changelogMatch) {
      versions["CHANGELOG.md"] = changelogMatch[1];
    } else {
      fail("CHANGELOG.md: could not find a version header (## [X.Y.Z])");
    }
  } else {
    fail("CHANGELOG.md not found");
  }

  // Compare all versions
  const sources = Object.keys(versions);
  if (sources.length < 3) {
    fail("Could not extract versions from all three sources");
    return;
  }

  const uniqueVersions = new Set(Object.values(versions));
  if (uniqueVersions.size === 1) {
    const ver = [...uniqueVersions][0];
    pass(`All versions match: ${ver}`);
    for (const src of sources) {
      pass(`  ${src} = ${versions[src]}`);
    }
  } else {
    fail("Version mismatch detected:");
    for (const src of sources) {
      console.log(`    ${src} = ${versions[src]}`);
    }
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

console.log("╔══════════════════════════════════════════════════╗");
console.log("║       Cortex Release Readiness Verification     ║");
console.log("╚══════════════════════════════════════════════════╝");

checkArchiveNaming();
checkDocLinks();
checkVersionConsistency();

console.log("\n──────────────────────────────────────────────────");
if (failures === 0) {
  console.log(`\n  ✓ ALL CHECKS PASSED\n`);
  process.exit(0);
} else {
  console.log(`\n  ✗ ${failures} CHECK(S) FAILED\n`);
  process.exit(1);
}
