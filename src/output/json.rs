use crate::diff::change::DiffResult;
use crate::repo::RepoAnalysis;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct FullOutput<'a> {
    diff: &'a DiffResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_analysis: Option<&'a RepoAnalysis>,
}

pub fn print_json(result: &DiffResult, repo_analysis: Option<&RepoAnalysis>) -> Result<()> {
    let output = FullOutput {
        diff: result,
        repo_analysis,
    };
    let json = serde_json::to_string_pretty(&output)?;
    println!("{}", json);
    Ok(())
}
