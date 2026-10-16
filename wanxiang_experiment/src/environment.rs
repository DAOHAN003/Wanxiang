//! Environment protocol for consumers that interact with a running Wanxiang world.
//!
//! Wanxiang is the environment. An Agent, script, user, or scheduler may be a
//! participant that submits actions through this protocol; none is required for
//! the world model itself to run.

use serde_json::Value;
use std::error::Error;
use std::fmt;
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

use crate::MAX_EXPERIMENT_TICKS;

pub trait WorldEnvironment {
    type Action;
    type Observation;
    type Error;

    fn reset(&mut self, seed: u64) -> Result<(), Self::Error>;
    fn observe(&self) -> Result<Self::Observation, Self::Error>;
    fn act(&mut self, action: Self::Action) -> Result<(), Self::Error>;
    fn current_tick(&self) -> u64;
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorldAction {
    Advance { ticks: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvironmentError {
    InvalidTicks(u64),
    InvalidObservation(String),
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTicks(ticks) => write!(f, "advance ticks must be in 1..={MAX_EXPERIMENT_TICKS}, got {ticks}"),
            Self::InvalidObservation(message) => write!(f, "could not serialize world observation: {message}"),
        }
    }
}

impl Error for EnvironmentError {}

/// First-party adapter around the existing seeded Rust SDK world.
pub struct SdkWorldEnvironment {
    world: WorldModel,
}

impl SdkWorldEnvironment {
    pub fn new(seed: u64) -> Self {
        Self {
            world: WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), seed),
        }
    }

    pub fn into_world(self) -> WorldModel {
        self.world
    }
}

impl WorldEnvironment for SdkWorldEnvironment {
    type Action = WorldAction;
    type Observation = Value;
    type Error = EnvironmentError;

    fn reset(&mut self, seed: u64) -> Result<(), Self::Error> {
        self.world = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), seed);
        Ok(())
    }

    fn observe(&self) -> Result<Self::Observation, Self::Error> {
        serde_json::from_str(&self.world.snapshot_json())
            .map_err(|error| EnvironmentError::InvalidObservation(error.to_string()))
    }

    fn act(&mut self, action: Self::Action) -> Result<(), Self::Error> {
        match action {
            WorldAction::Advance { ticks } if (1..=MAX_EXPERIMENT_TICKS).contains(&ticks) => {
                self.world.step(ticks);
                Ok(())
            }
            WorldAction::Advance { ticks } => Err(EnvironmentError::InvalidTicks(ticks)),
        }
    }

    fn current_tick(&self) -> u64 {
        self.world.current_tick()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_can_reset_observe_and_advance() {
        let mut environment = SdkWorldEnvironment::new(11);
        let initial = environment.observe().expect("initial snapshot should be valid");
        assert_eq!(initial["meta"]["seed"].as_u64(), Some(11));
        assert_eq!(environment.current_tick(), 0);

        environment.act(WorldAction::Advance { ticks: 3 }).expect("valid action should advance");
        assert_eq!(environment.current_tick(), 3);
        let later = environment.observe().expect("snapshot should remain valid");
        assert_eq!(later["meta"]["tick"].as_u64(), Some(3));

        environment.reset(42).expect("reset should succeed");
        let reset = environment.observe().expect("reset snapshot should be valid");
        assert_eq!(reset["meta"]["seed"].as_u64(), Some(42));
        assert_eq!(environment.current_tick(), 0);
    }

    #[test]
    fn environment_rejects_out_of_bounds_actions() {
        let mut environment = SdkWorldEnvironment::new(1);
        assert!(matches!(
            environment.act(WorldAction::Advance { ticks: 0 }),
            Err(EnvironmentError::InvalidTicks(0))
        ));
        assert_eq!(environment.current_tick(), 0);
    }
}
