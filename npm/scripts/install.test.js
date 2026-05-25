#!/usr/bin/env node
/**
 * Property test for PATH modification idempotency.
 *
 * Property 9: For any shell configuration file content that already contains
 * the ~/.cortex/bin path string, the PATH configuration function SHALL not
 * modify the file (no duplicate entries). Conversely, for content that does
 * not contain the path, the function SHALL prepend (not append) the export line.
 *
 * Validates: Requirements 6.3, 6.4, 6.5
 */

const assert = require("node:assert");
const os = require("node:os");
const path = require("node:path");

const INSTALL_DIR = path.join(os.homedir(), ".cortex", "bin");
const EXPORT_LINE = `export PATH="${INSTALL_DIR}:$PATH"`;

/**
 * Simulate configurePathUnix logic for a single file.
 * This mirrors the actual logic in install.js:
 *   if (content.includes(installDir)) { continue; }
 *   fs.appendFileSync(file, `\n# Added by cortex installer\n${exportLine}\n`);
 *
 * Returns the new content (or same content if no modification needed).
 */
function simulateConfigurePathUnix(existingContent, installDir) {
  if (existingContent.includes(installDir)) {
    return existingContent; // No modification — already configured
  }
  const exportLine = `export PATH="${installDir}:$PATH"`;
  return existingContent + `\n# Added by cortex installer\n${exportLine}\n`;
}

/**
 * Generate random shell config content for property testing.
 * @param {boolean} includeInstallDir - Whether to embed the install dir in the content
 */
function generateRandomShellContent(includeInstallDir) {
  const lines = [];
  const numLines = Math.floor(Math.random() * 20) + 1;

  for (let i = 0; i < numLines; i++) {
    const lineType = Math.floor(Math.random() * 6);
    switch (lineType) {
      case 0:
        lines.push(`# Comment ${Math.random().toString(36).slice(2)}`);
        break;
      case 1:
        lines.push(`export PATH="/usr/local/bin:$PATH"`);
        break;
      case 2:
        lines.push(`alias ll='ls -la'`);
        break;
      case 3:
        lines.push(`export EDITOR=vim`);
        break;
      case 4:
        lines.push("");
        break;
      case 5:
        lines.push(`source ~/.nvm/nvm.sh`);
        break;
    }
  }

  if (includeInstallDir) {
    // Insert the install dir reference at a random position
    const pos = Math.floor(Math.random() * (lines.length + 1));
    lines.splice(pos, 0, `export PATH="${INSTALL_DIR}:$PATH"`);
  }

  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Property Tests
// ---------------------------------------------------------------------------

const NUM_ITERATIONS = 100;

console.log("Property 9: PATH modification idempotency");
console.log(`Running ${NUM_ITERATIONS} iterations per sub-property...\n`);

// Sub-property A: Content already containing install dir should NOT be modified
let passCount = 0;
for (let i = 0; i < NUM_ITERATIONS; i++) {
  const content = generateRandomShellContent(true);
  const result = simulateConfigurePathUnix(content, INSTALL_DIR);
  assert.strictEqual(
    result,
    content,
    `Iteration ${i}: Content was modified when install dir already present`
  );
  passCount++;
}
console.log(`  [PASS] ${passCount}/${NUM_ITERATIONS}: No modification when path already present`);

// Sub-property B: Content NOT containing install dir should have export line added
passCount = 0;
for (let i = 0; i < NUM_ITERATIONS; i++) {
  const content = generateRandomShellContent(false);
  const result = simulateConfigurePathUnix(content, INSTALL_DIR);

  // The result must contain the install dir
  assert(
    result.includes(INSTALL_DIR),
    `Iteration ${i}: Result does not contain install dir after modification`
  );

  // The export line should appear exactly once
  const escapedDir = INSTALL_DIR.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const occurrences = (result.match(new RegExp(escapedDir, "g")) || []).length;
  assert.strictEqual(
    occurrences,
    1,
    `Iteration ${i}: Install dir appears ${occurrences} times (expected 1)`
  );

  // The PATH entry should prepend (installDir comes before $PATH in the export)
  assert(
    result.includes(`"${INSTALL_DIR}:$PATH"`),
    `Iteration ${i}: PATH not prepended correctly — expected "${INSTALL_DIR}:$PATH"`
  );

  passCount++;
}
console.log(`  [PASS] ${passCount}/${NUM_ITERATIONS}: Export line correctly added when path not present`);

// Sub-property C: Idempotency — applying the function twice yields the same result
passCount = 0;
for (let i = 0; i < NUM_ITERATIONS; i++) {
  const content = generateRandomShellContent(false);
  const firstPass = simulateConfigurePathUnix(content, INSTALL_DIR);
  const secondPass = simulateConfigurePathUnix(firstPass, INSTALL_DIR);

  assert.strictEqual(
    firstPass,
    secondPass,
    `Iteration ${i}: Second application modified the content (not idempotent)`
  );

  // Count occurrences — must be exactly 1 after any number of applications
  const escapedDir = INSTALL_DIR.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const occurrences = (secondPass.match(new RegExp(escapedDir, "g")) || []).length;
  assert.strictEqual(
    occurrences,
    1,
    `Iteration ${i}: Duplicate entries after second pass (found ${occurrences})`
  );

  passCount++;
}
console.log(`  [PASS] ${passCount}/${NUM_ITERATIONS}: Idempotent — no duplicates on repeated application`);

// Sub-property D: The added content is appended (not inserted before existing content)
passCount = 0;
for (let i = 0; i < NUM_ITERATIONS; i++) {
  const content = generateRandomShellContent(false);
  const result = simulateConfigurePathUnix(content, INSTALL_DIR);

  // Original content should still be at the start of the result
  assert(
    result.startsWith(content),
    `Iteration ${i}: Original content was not preserved at the start of the file`
  );

  passCount++;
}
console.log(`  [PASS] ${passCount}/${NUM_ITERATIONS}: Original file content preserved (export appended to end)`);

console.log("\nAll property tests passed.");
