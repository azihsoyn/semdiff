use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub summary: String,
    pub risk_level: RiskLevel,
    pub key_observations: Vec<String>,
    pub potential_issues: Vec<ReviewIssue>,
    pub test_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "LOW"),
            RiskLevel::Medium => write!(f, "MEDIUM"),
            RiskLevel::High => write!(f, "HIGH"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub severity: RiskLevel,
    pub description: String,
    pub suggestion: Option<String>,
}

impl Default for ReviewResult {
    fn default() -> Self {
        Self {
            summary: "Review not yet performed.".to_string(),
            risk_level: RiskLevel::Low,
            key_observations: Vec::new(),
            potential_issues: Vec::new(),
            test_suggestions: Vec::new(),
        }
    }
}
