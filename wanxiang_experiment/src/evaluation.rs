//! Typed trial records and paired comparisons for Agent experiments.
//!
//! This module does not run Agents itself. Adapters should record real task
//! outcomes here; comparisons require identical (task_id, seed) pairs.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTrialRecord {
    pub agent_id: String,
    pub task_id: String,
    pub seed: u64,
    pub success: bool,
    pub action_count: u64,
    pub tool_errors: u64,
    /// Application-defined cost units, such as model tokens or simulated resources.
    pub cost_units: f64,
}

impl AgentTrialRecord {
    pub fn validate(&self) -> Result<(), AgentEvaluationError> {
        if self.agent_id.trim().is_empty() {
            return Err(AgentEvaluationError::EmptyAgentId);
        }
        if self.task_id.trim().is_empty() {
            return Err(AgentEvaluationError::EmptyTaskId);
        }
        if !self.cost_units.is_finite() || self.cost_units < 0.0 {
            return Err(AgentEvaluationError::InvalidCost(self.cost_units));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub agent_id: String,
    pub run_count: u64,
    pub successes: u64,
    pub success_rate: f64,
    pub mean_actions: f64,
    pub total_tool_errors: u64,
    pub mean_cost_units: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairedAgentComparison {
    pub task_seed_pairs: u64,
    pub baseline: AgentSummary,
    pub candidate: AgentSummary,
    /// Candidate minus baseline; positive values mean more successes, actions, errors, or cost.
    pub success_rate_delta: f64,
    pub mean_actions_delta: f64,
    pub tool_errors_delta: i64,
    pub mean_cost_units_delta: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvaluationError {
    EmptyAgentId,
    EmptyTaskId,
    InvalidCost(f64),
    DuplicateTrial { agent: String, task: String, seed: u64 },
    SameAgent,
    MissingAgent(String),
    NoTrials,
    UnpairedTaskSeed { task: String, seed: u64 },
}

impl fmt::Display for AgentEvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAgentId => write!(f, "agent_id must not be empty"),
            Self::EmptyTaskId => write!(f, "task_id must not be empty"),
            Self::InvalidCost(value) => write!(f, "cost_units must be finite and non-negative, got {value}"),
            Self::DuplicateTrial { agent, task, seed } => write!(f, "duplicate trial for agent={agent}, task={task}, seed={seed}"),
            Self::SameAgent => write!(f, "baseline and candidate agent ids must differ"),
            Self::MissingAgent(agent) => write!(f, "no trial records for agent {agent}"),
            Self::NoTrials => write!(f, "at least one trial record is required"),
            Self::UnpairedTaskSeed { task, seed } => write!(f, "baseline and candidate do not share task={task}, seed={seed}"),
        }
    }
}

impl Error for AgentEvaluationError {}

fn index_trials<'a>(
    records: &'a [AgentTrialRecord],
) -> Result<BTreeMap<(&'a str, &'a str, u64), &'a AgentTrialRecord>, AgentEvaluationError> {
    if records.is_empty() {
        return Err(AgentEvaluationError::NoTrials);
    }
    let mut indexed = BTreeMap::new();
    for record in records {
        record.validate()?;
        let key = (record.agent_id.as_str(), record.task_id.as_str(), record.seed);
        if indexed.insert(key, record).is_some() {
            return Err(AgentEvaluationError::DuplicateTrial {
                agent: record.agent_id.clone(),
                task: record.task_id.clone(),
                seed: record.seed,
            });
        }
    }
    Ok(indexed)
}

fn summarize_agent<'a>(
    agent_id: &str,
    records: impl Iterator<Item = &'a AgentTrialRecord>,
) -> Result<AgentSummary, AgentEvaluationError> {
    let chosen: Vec<&AgentTrialRecord> = records.filter(|record| record.agent_id == agent_id).collect();
    if chosen.is_empty() {
        return Err(AgentEvaluationError::MissingAgent(agent_id.into()));
    }
    let count = chosen.len() as u64;
    let successes = chosen.iter().filter(|record| record.success).count() as u64;
    let total_actions: u64 = chosen.iter().map(|record| record.action_count).sum();
    let total_errors: u64 = chosen.iter().map(|record| record.tool_errors).sum();
    let total_cost: f64 = chosen.iter().map(|record| record.cost_units).sum();

    Ok(AgentSummary {
        agent_id: agent_id.into(),
        run_count: count,
        successes,
        success_rate: successes as f64 / count as f64,
        mean_actions: total_actions as f64 / count as f64,
        total_tool_errors: total_errors,
        mean_cost_units: total_cost / count as f64,
    })
}

/// Aggregate one or more agents' records into deterministic, name-sorted summaries.
pub fn summarize_agent_trials(
    records: &[AgentTrialRecord],
) -> Result<Vec<AgentSummary>, AgentEvaluationError> {
    index_trials(records)?;
    let ids: BTreeSet<&str> = records.iter().map(|record| record.agent_id.as_str()).collect();
    ids.into_iter()
        .map(|agent_id| summarize_agent(agent_id, records.iter()))
        .collect()
}

/// Compare two agents only when each has one record for every same task/seed pair.
pub fn compare_agents(
    records: &[AgentTrialRecord],
    baseline_agent: &str,
    candidate_agent: &str,
) -> Result<PairedAgentComparison, AgentEvaluationError> {
    if baseline_agent == candidate_agent {
        return Err(AgentEvaluationError::SameAgent);
    }
    let indexed = index_trials(records)?;
    let baseline_pairs: BTreeSet<(&str, u64)> = indexed
        .keys()
        .filter_map(|(agent, task, seed)| (*agent == baseline_agent).then_some((*task, *seed)))
        .collect();
    let candidate_pairs: BTreeSet<(&str, u64)> = indexed
        .keys()
        .filter_map(|(agent, task, seed)| (*agent == candidate_agent).then_some((*task, *seed)))
        .collect();

    if baseline_pairs.is_empty() {
        return Err(AgentEvaluationError::MissingAgent(baseline_agent.into()));
    }
    if candidate_pairs.is_empty() {
        return Err(AgentEvaluationError::MissingAgent(candidate_agent.into()));
    }
    for (task, seed) in baseline_pairs.symmetric_difference(&candidate_pairs) {
        return Err(AgentEvaluationError::UnpairedTaskSeed {
            task: (*task).into(),
            seed: *seed,
        });
    }

    let baseline = summarize_agent(baseline_agent, records.iter())?;
    let candidate = summarize_agent(candidate_agent, records.iter())?;
    Ok(PairedAgentComparison {
        task_seed_pairs: baseline_pairs.len() as u64,
        success_rate_delta: candidate.success_rate - baseline.success_rate,
        mean_actions_delta: candidate.mean_actions - baseline.mean_actions,
        tool_errors_delta: candidate.total_tool_errors as i64 - baseline.total_tool_errors as i64,
        mean_cost_units_delta: candidate.mean_cost_units - baseline.mean_cost_units,
        baseline,
        candidate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(agent: &str, task: &str, seed: u64, success: bool, actions: u64, cost: f64) -> AgentTrialRecord {
        AgentTrialRecord {
            agent_id: agent.into(),
            task_id: task.into(),
            seed,
            success,
            action_count: actions,
            tool_errors: u64::from(!success),
            cost_units: cost,
        }
    }

    #[test]
    fn paired_comparison_uses_identical_task_seed_pairs() {
        let records = vec![
            trial("baseline", "survive", 1, false, 5, 2.0),
            trial("candidate", "survive", 1, true, 3, 3.0),
            trial("baseline", "trade", 2, true, 4, 1.0),
            trial("candidate", "trade", 2, true, 3, 2.0),
        ];
        let comparison = compare_agents(&records, "baseline", "candidate").expect("paired comparison should work");
        assert_eq!(comparison.task_seed_pairs, 2);
        assert_eq!(comparison.baseline.success_rate, 0.5);
        assert_eq!(comparison.candidate.success_rate, 1.0);
        assert_eq!(comparison.success_rate_delta, 0.5);
        assert_eq!(comparison.mean_actions_delta, -1.5);
    }

    #[test]
    fn rejects_unpaired_and_duplicate_trials() {
        let unpaired = vec![
            trial("baseline", "survive", 1, false, 5, 2.0),
            trial("candidate", "trade", 1, true, 3, 3.0),
        ];
        assert!(matches!(
            compare_agents(&unpaired, "baseline", "candidate"),
            Err(AgentEvaluationError::UnpairedTaskSeed { .. })
        ));

        let duplicated = vec![
            trial("baseline", "survive", 1, false, 5, 2.0),
            trial("baseline", "survive", 1, true, 4, 1.0),
        ];
        assert!(matches!(
            summarize_agent_trials(&duplicated),
            Err(AgentEvaluationError::DuplicateTrial { .. })
        ));
    }

    #[test]
    fn rejects_invalid_costs() {
        let records = vec![trial("agent", "task", 1, true, 1, f64::NAN)];
        assert!(matches!(
            summarize_agent_trials(&records),
            Err(AgentEvaluationError::InvalidCost(_))
        ));
    }
}
