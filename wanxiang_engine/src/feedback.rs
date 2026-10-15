//! Deterministic, bounded feedback from learned event patterns into world dynamics.
//!
//! This controller deliberately adjusts only the ecosystem-health state, which the next
//! LifeLayer tick already evolves. It does not rewrite the user's WorldRules or RNG state.

use serde::{Deserialize, Serialize};
use wanxiang_evolution::learning::LearnResult;
use wanxiang_types::{EventImportance, WorldEvent};

const MAX_STRESS: f64 = 0.02;
const MAX_RESILIENCE: f64 = 0.005;
const MAX_LEARNING_SIGNALS: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveFeedback {
    pub event_count: usize,
    pub mean_event_importance: f64,
    pub learned_signal_count: usize,
    pub delta: f64,
    pub ecosystem_health_before: f64,
    pub ecosystem_health_after: f64,
}

/// A small bounded policy: high-importance events create stress; learned patterns/inferences
/// provide a smaller resilience term. This is a deterministic simulation rule, not a claim
/// that text-derived patterns are scientifically valid causal knowledge.
pub fn apply_event_learning_feedback(
    events: &[WorldEvent],
    learning: &LearnResult,
    ecosystem_health: &mut f64,
) -> Option<AdaptiveFeedback> {
    if events.is_empty() {
        return None;
    }

    let mean_event_importance = events
        .iter()
        .map(|event| event.importance as u8 as f64)
        .sum::<f64>()
        / events.len() as f64;
    let learned_signal_count = learning
        .patterns
        .len()
        .saturating_add(learning.inferences.len())
        .min(MAX_LEARNING_SIGNALS);
    let stress = (mean_event_importance / EventImportance::WorldShaking as u8 as f64)
        * MAX_STRESS;
    let resilience = (learned_signal_count as f64 / MAX_LEARNING_SIGNALS as f64)
        * MAX_RESILIENCE;
    let delta = (resilience - stress).clamp(-MAX_STRESS, MAX_RESILIENCE);
    let before = ecosystem_health.clamp(0.0, 1.0);
    let after = (before + delta).clamp(0.0, 1.0);
    *ecosystem_health = after;

    Some(AdaptiveFeedback {
        event_count: events.len(),
        mean_event_importance,
        learned_signal_count,
        delta: after - before,
        ecosystem_health_before: before,
        ecosystem_health_after: after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wanxiang_types::{EventType, WorldEvent};

    fn event(importance: EventImportance, tick: u64) -> WorldEvent {
        WorldEvent::new(
            tick,
            EventType::Emergence,
            importance,
            format!("test event at tick {tick}"),
        )
    }

    #[test]
    fn feedback_is_bounded_and_changes_next_tick_state() {
        let events = vec![
            event(EventImportance::Critical, 1),
            event(EventImportance::Major, 1),
        ];
        let learning = LearnResult::default();
        let mut health = 0.5;
        let feedback = apply_event_learning_feedback(&events, &learning, &mut health).unwrap();
        assert!(feedback.delta >= -MAX_STRESS && feedback.delta <= MAX_RESILIENCE);
        assert_eq!(health, feedback.ecosystem_health_after);
        assert!(health < 0.5, "high-importance events should create bounded stress");
    }

    #[test]
    fn learned_patterns_add_a_bounded_resilience_signal() {
        let events = vec![event(EventImportance::Notable, 2)];
        let no_learning = LearnResult::default();
        let mut baseline = 0.5;
        apply_event_learning_feedback(&events, &no_learning, &mut baseline);

        let mut learned = LearnResult::default();
        learned.patterns.push(Default::default());
        learned.inferences.push(Default::default());
        let mut with_learning = 0.5;
        let feedback = apply_event_learning_feedback(&events, &learned, &mut with_learning).unwrap();

        assert!(with_learning > baseline);
        assert_eq!(feedback.learned_signal_count, 2);
        assert!(feedback.delta <= MAX_RESILIENCE);
    }

    #[test]
    fn empty_event_batch_does_not_mutate_state() {
        let mut health = 0.4;
        assert!(apply_event_learning_feedback(&[], &LearnResult::default(), &mut health).is_none());
        assert_eq!(health, 0.4);
    }

    #[test]
    fn feedback_clamps_health_at_physical_bounds() {
        let events = vec![event(EventImportance::WorldShaking, 3)];
        let mut health = 0.0;
        let result = apply_event_learning_feedback(&events, &LearnResult::default(), &mut health).unwrap();
        assert_eq!(health, 0.0);
        assert_eq!(result.ecosystem_health_after, 0.0);
    }
}
