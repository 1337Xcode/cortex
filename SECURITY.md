# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| 0.0.x   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in Cortex, please report it responsibly.

### How to Report

1. **Do NOT** open a public GitHub issue for security vulnerabilities.
2. Email your report to: me@darshanchheda.com
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

### What to Expect

- **Acknowledgment**: Within 48 hours of your report
- **Assessment**: Within 7 days, we will assess the severity and impact
- **Fix timeline**: Critical vulnerabilities will be patched within 14 days
- **Disclosure**: We follow coordinated disclosure. We will work with you on timing.

### Scope

The following are in scope for security reports:

- Remote code execution via MCP tool calls
- SQL injection in graph queries
- Path traversal in file operations
- Information disclosure of sensitive file contents
- Denial of service via resource exhaustion
- Authentication/authorization bypass (if applicable)

### Out of Scope

- Vulnerabilities in dependencies (report to upstream maintainers)
- Issues requiring physical access to the machine
- Social engineering attacks
- Denial of service via legitimate heavy usage

## Security Design

Cortex is designed with the following security principles:

1. **Local-only by default**: MCP communication is via stdio, no network ports opened unless explicitly requested
2. **Read-only graph access**: MCP queries use read-only database connections
3. **No code execution**: Cortex never executes code from indexed repositories
4. **No secrets in logs**: Tracing never logs file contents, observation text, or source code
5. **Input sanitization**: All FTS5 queries are sanitized to prevent injection
6. **Air-gap compatible**: All features work without network access (except OSV.dev checks and `cortex update`)
7. **SHA-256 verified updates**: `cortex update` and the npm installer verify checksums before replacing binaries
8. **Stale binary cleanup**: Installer removes old binaries from other PATH locations to prevent version confusion
