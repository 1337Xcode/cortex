//! Security analysis module.
//!
//! Provides taint analysis, OWASP Top 10 pattern detection,
//! CWE classification, SBOM generation, vulnerability checking,
//! and secret detection/redaction.

pub mod cwe;
pub mod owasp;
pub mod sbom;
pub mod secrets;
pub mod taint;
pub mod vuln;
