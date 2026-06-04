//! Confidence tiers and edge source types for graph edges.
//!
//! Edges are tagged with a confidence level (0.0–1.0) and a source that
//! indicates which analysis step produced them. Higher confidence sources
//! supersede lower ones for the same (source_fqn, target_fqn) pair.

use serde::{Deserialize, Serialize};

/// Confidence tier mapped to a numeric value used in edge queries.
///
/// | Tier      | Value | Source                        |
/// |-----------|-------|-------------------------------|
/// | High      | 1.0   | SCIP precise resolution       |
/// | Medium    | 0.8   | Framework adapter patterns    |
/// | Low       | 0.5   | AST heuristics (tree-sitter)  |
/// | VeryLow   | 0.2   | Name-match / speculative      |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidenceTier {
    /// Precise resolution from SCIP index. Confidence = 1.0.
    High,
    /// Framework-adapter pattern match. Confidence = 0.8.
    Medium,
    /// AST-derived heuristic edge. Confidence = 0.5.
    Low,
    /// Name-match or speculative edge. Confidence = 0.2.
    VeryLow,
}

impl ConfidenceTier {
    /// Return the numeric confidence value for this tier.
    pub fn numeric(self) -> f64 {
        match self {
            Self::High => 1.0,
            Self::Medium => 0.8,
            Self::Low => 0.5,
            Self::VeryLow => 0.2,
        }
    }

    /// Parse a confidence tier from a numeric value (nearest tier).
    pub fn from_f64(v: f64) -> Self {
        if v >= 0.9 {
            Self::High
        } else if v >= 0.65 {
            Self::Medium
        } else if v >= 0.35 {
            Self::Low
        } else {
            Self::VeryLow
        }
    }
}

impl std::fmt::Display for ConfidenceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
            Self::VeryLow => "VERY_LOW",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// EdgeSource
// ---------------------------------------------------------------------------

/// The source system that produced an edge.
///
/// Stored as the `edge_source` column value in the `edges` table.
/// Maps to the CHECK constraint: `('scip', 'framework_adapter', 'ast_direct', 'name_match')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeSource {
    /// Precise symbol resolution from a SCIP index file.
    Scip,
    /// Pattern-matched from a framework adapter (DI, routing, middleware).
    FrameworkAdapter,
    /// Heuristic edge derived from AST parsing (tree-sitter).
    AstDirect,
    /// Speculative edge based on name matching.
    NameMatch,
}

impl EdgeSource {
    /// Return the database string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scip => "scip",
            Self::FrameworkAdapter => "framework_adapter",
            Self::AstDirect => "ast_direct",
            Self::NameMatch => "name_match",
        }
    }

    /// Parse from a database string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "scip" => Some(Self::Scip),
            "framework_adapter" => Some(Self::FrameworkAdapter),
            "ast_direct" => Some(Self::AstDirect),
            "name_match" => Some(Self::NameMatch),
            _ => None,
        }
    }

    /// Return the default confidence tier for this edge source.
    pub fn default_confidence(self) -> ConfidenceTier {
        match self {
            Self::Scip => ConfidenceTier::High,
            Self::FrameworkAdapter => ConfidenceTier::Medium,
            Self::AstDirect => ConfidenceTier::Low,
            Self::NameMatch => ConfidenceTier::VeryLow,
        }
    }
}

impl std::fmt::Display for EdgeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_tier_numeric_values() {
        assert!((ConfidenceTier::High.numeric() - 1.0).abs() < f64::EPSILON);
        assert!((ConfidenceTier::Medium.numeric() - 0.8).abs() < f64::EPSILON);
        assert!((ConfidenceTier::Low.numeric() - 0.5).abs() < f64::EPSILON);
        assert!((ConfidenceTier::VeryLow.numeric() - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn confidence_tier_from_f64() {
        assert_eq!(ConfidenceTier::from_f64(1.0), ConfidenceTier::High);
        assert_eq!(ConfidenceTier::from_f64(0.95), ConfidenceTier::High);
        assert_eq!(ConfidenceTier::from_f64(0.9), ConfidenceTier::High);
        assert_eq!(ConfidenceTier::from_f64(0.8), ConfidenceTier::Medium);
        assert_eq!(ConfidenceTier::from_f64(0.65), ConfidenceTier::Medium);
        assert_eq!(ConfidenceTier::from_f64(0.5), ConfidenceTier::Low);
        assert_eq!(ConfidenceTier::from_f64(0.35), ConfidenceTier::Low);
        assert_eq!(ConfidenceTier::from_f64(0.2), ConfidenceTier::VeryLow);
        assert_eq!(ConfidenceTier::from_f64(0.0), ConfidenceTier::VeryLow);
    }

    #[test]
    fn edge_source_as_str() {
        assert_eq!(EdgeSource::Scip.as_str(), "scip");
        assert_eq!(EdgeSource::FrameworkAdapter.as_str(), "framework_adapter");
        assert_eq!(EdgeSource::AstDirect.as_str(), "ast_direct");
        assert_eq!(EdgeSource::NameMatch.as_str(), "name_match");
    }

    #[test]
    fn edge_source_from_str_roundtrip() {
        for source in [
            EdgeSource::Scip,
            EdgeSource::FrameworkAdapter,
            EdgeSource::AstDirect,
            EdgeSource::NameMatch,
        ] {
            let s = source.as_str();
            let parsed = EdgeSource::from_str(s);
            assert_eq!(parsed, Some(source), "from_str failed for '{}'", s);
        }
    }

    #[test]
    fn edge_source_from_str_unknown() {
        assert_eq!(EdgeSource::from_str("unknown"), None);
        assert_eq!(EdgeSource::from_str(""), None);
        assert_eq!(EdgeSource::from_str("SCIP"), None); // case-sensitive
    }

    #[test]
    fn edge_source_default_confidence() {
        assert_eq!(
            EdgeSource::Scip.default_confidence(),
            ConfidenceTier::High
        );
        assert_eq!(
            EdgeSource::FrameworkAdapter.default_confidence(),
            ConfidenceTier::Medium
        );
        assert_eq!(
            EdgeSource::AstDirect.default_confidence(),
            ConfidenceTier::Low
        );
        assert_eq!(
            EdgeSource::NameMatch.default_confidence(),
            ConfidenceTier::VeryLow
        );
    }
}
