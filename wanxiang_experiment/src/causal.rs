//! A small, explicitly specified linear structural causal model (SCM).
//!
//! This module is a transparent reference implementation for known-answer tests
//! and model-internal interventions. It is not causal discovery and makes no
//! claims about real-world validity beyond the equations supplied by the caller.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearScmNode {
    pub name: String,
    pub intercept: f64,
    pub exogenous: f64,
    /// Parent variable name -> linear structural coefficient.
    pub parents: BTreeMap<String, f64>,
}

impl LinearScmNode {
    pub fn new(
        name: impl Into<String>,
        intercept: f64,
        exogenous: f64,
        parents: BTreeMap<String, f64>,
    ) -> Self {
        Self { name: name.into(), intercept, exogenous, parents }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearScm {
    nodes: BTreeMap<String, LinearScmNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CausalInterventionResult {
    pub target: String,
    pub assigned_value: f64,
    pub baseline: BTreeMap<String, f64>,
    pub intervened: BTreeMap<String, f64>,
    /// Intervened value minus baseline value for each variable.
    pub deltas: BTreeMap<String, f64>,
    pub interpretation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CausalModelError {
    EmptyVariableName,
    DuplicateVariable(String),
    UnknownParent { variable: String, parent: String },
    NonFiniteValue { variable: String, field: String },
    UnknownIntervention(String),
    NonFiniteIntervention(f64),
    Cycle(Vec<String>),
}

impl fmt::Display for CausalModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyVariableName => write!(f, "variable names must not be empty"),
            Self::DuplicateVariable(name) => write!(f, "duplicate variable: {name}"),
            Self::UnknownParent { variable, parent } => write!(f, "variable {variable} references unknown parent {parent}"),
            Self::NonFiniteValue { variable, field } => write!(f, "variable {variable} has a non-finite {field}"),
            Self::UnknownIntervention(name) => write!(f, "cannot intervene on unknown variable: {name}"),
            Self::NonFiniteIntervention(value) => write!(f, "intervention value must be finite, got {value}"),
            Self::Cycle(names) => write!(f, "structural equations contain a directed cycle involving: {}", names.join(", ")),
        }
    }
}

impl Error for CausalModelError {}

impl LinearScm {
    pub fn new(nodes: Vec<LinearScmNode>) -> Result<Self, CausalModelError> {
        let mut by_name = BTreeMap::new();
        for node in nodes {
            if node.name.trim().is_empty() {
                return Err(CausalModelError::EmptyVariableName);
            }
            if by_name.contains_key(&node.name) {
                return Err(CausalModelError::DuplicateVariable(node.name));
            }
            if !node.intercept.is_finite() {
                return Err(CausalModelError::NonFiniteValue {
                    variable: node.name.clone(),
                    field: "intercept".into(),
                });
            }
            if !node.exogenous.is_finite() {
                return Err(CausalModelError::NonFiniteValue {
                    variable: node.name.clone(),
                    field: "exogenous value".into(),
                });
            }
            for (parent, coefficient) in &node.parents {
                if !coefficient.is_finite() {
                    return Err(CausalModelError::NonFiniteValue {
                        variable: node.name.clone(),
                        field: format!("coefficient for parent {parent}"),
                    });
                }
            }
            by_name.insert(node.name.clone(), node);
        }

        for node in by_name.values() {
            for parent in node.parents.keys() {
                if !by_name.contains_key(parent) {
                    return Err(CausalModelError::UnknownParent {
                        variable: node.name.clone(),
                        parent: parent.clone(),
                    });
                }
            }
        }

        let model = Self { nodes: by_name };
        model.topological_order()?;
        Ok(model)
    }

    pub fn variable_names(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    /// Evaluate the structural equations in deterministic topological order.
    pub fn evaluate(&self) -> Result<BTreeMap<String, f64>, CausalModelError> {
        self.evaluate_internal(None)
    }

    /// Apply a do-intervention by replacing the target variable's equation with
    /// the assigned value, then recomputing all descendants.
    pub fn intervene(
        &self,
        target: &str,
        assigned_value: f64,
    ) -> Result<CausalInterventionResult, CausalModelError> {
        if !self.nodes.contains_key(target) {
            return Err(CausalModelError::UnknownIntervention(target.into()));
        }
        if !assigned_value.is_finite() {
            return Err(CausalModelError::NonFiniteIntervention(assigned_value));
        }

        let baseline = self.evaluate_internal(None)?;
        let intervened = self.evaluate_internal(Some((target, assigned_value)))?;
        let deltas = self
            .nodes
            .keys()
            .map(|name| {
                let before = baseline[name];
                let after = intervened[name];
                (name.clone(), after - before)
            })
            .collect();

        Ok(CausalInterventionResult {
            target: target.into(),
            assigned_value,
            baseline,
            intervened,
            deltas,
            interpretation: "Effect within the supplied deterministic linear SCM only; validity depends on the caller's structural equations and assumptions.".into(),
        })
    }

    fn evaluate_internal(
        &self,
        intervention: Option<(&str, f64)>,
    ) -> Result<BTreeMap<String, f64>, CausalModelError> {
        let order = self.topological_order()?;
        let mut values = BTreeMap::new();
        for name in order {
            let node = &self.nodes[&name];
            let value = match intervention {
                Some((target, assigned_value)) if target == name => assigned_value,
                _ => {
                    let parent_sum: f64 = node.parents
                        .iter()
                        .map(|(parent, coefficient)| coefficient * values[parent])
                        .sum();
                    node.intercept + node.exogenous + parent_sum
                }
            };
            if !value.is_finite() {
                return Err(CausalModelError::NonFiniteValue {
                    variable: name,
                    field: "computed value".into(),
                });
            }
            values.insert(node.name.clone(), value);
        }
        Ok(values)
    }

    fn topological_order(&self) -> Result<Vec<String>, CausalModelError> {
        let mut indegree: BTreeMap<String, usize> = self
            .nodes
            .keys()
            .map(|name| (name.clone(), 0))
            .collect();
        let mut children: BTreeMap<String, Vec<String>> = self
            .nodes
            .keys()
            .map(|name| (name.clone(), Vec::new()))
            .collect();

        for node in self.nodes.values() {
            for parent in node.parents.keys() {
                *indegree.get_mut(&node.name).expect("node exists") += 1;
                children.get_mut(parent).expect("parent validated").push(node.name.clone());
            }
        }
        for descendants in children.values_mut() {
            descendants.sort();
        }

        let mut ready: Vec<String> = indegree
            .iter()
            .filter_map(|(name, degree)| (*degree == 0).then_some(name.clone()))
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while !ready.is_empty() {
            let name = ready.remove(0);
            order.push(name.clone());
            for child in &children[&name] {
                let degree = indegree.get_mut(child).expect("child exists");
                *degree -= 1;
                if *degree == 0 {
                    ready.push(child.clone());
                    ready.sort();
                }
            }
        }

        if order.len() != self.nodes.len() {
            let remaining = indegree
                .into_iter()
                .filter_map(|(name, degree)| (degree > 0).then_some(name))
                .collect();
            return Err(CausalModelError::Cycle(remaining));
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, exogenous: f64, parents: &[(&str, f64)]) -> LinearScmNode {
        LinearScmNode::new(
            name,
            0.0,
            exogenous,
            parents.iter().map(|(name, coefficient)| ((*name).to_string(), *coefficient)).collect(),
        )
    }

    #[test]
    fn intervention_recomputes_descendants_with_known_answer() {
        let model = LinearScm::new(vec![
            node("rain", 1.0, &[]),
            node("wet_grass", 0.0, &[("rain", 2.0)]),
        ]).expect("DAG should validate");

        let result = model.intervene("rain", 0.0).expect("target should exist");
        assert_eq!(result.baseline["rain"], 1.0);
        assert_eq!(result.baseline["wet_grass"], 2.0);
        assert_eq!(result.intervened["rain"], 0.0);
        assert_eq!(result.intervened["wet_grass"], 0.0);
        assert_eq!(result.deltas["wet_grass"], -2.0);
    }

    #[test]
    fn rejects_unknown_parent_and_cycles() {
        let unknown = LinearScm::new(vec![node("child", 0.0, &[("missing", 1.0)])]);
        assert!(matches!(unknown, Err(CausalModelError::UnknownParent { .. })));

        let cycle = LinearScm::new(vec![
            node("a", 0.0, &[("b", 1.0)]),
            node("b", 0.0, &[("a", 1.0)]),
        ]);
        assert!(matches!(cycle, Err(CausalModelError::Cycle(_))));
    }

    #[test]
    fn rejects_non_finite_equations_and_interventions() {
        assert!(matches!(
            LinearScm::new(vec![node("x", f64::NAN, &[])]),
            Err(CausalModelError::NonFiniteValue { .. })
        ));
        let model = LinearScm::new(vec![node("x", 1.0, &[])]).expect("model should be valid");
        assert!(matches!(
            model.intervene("x", f64::INFINITY),
            Err(CausalModelError::NonFiniteIntervention(_))
        ));
    }
}
