//! # wanxiang_sdk — 万象世界模型可嵌入 SDK（定位③：世界模型基座）
//!
//! 把 `LivingCore` / `WorldSimulator` 包装成稳定、最小、文档化的 `WorldModel` 公共 API，
//! 供下游（游戏后端 / 互动叙事 / 看板 / 研究脚本）嵌入使用。下游**只依赖本 crate**，
//! 不直接触碰内部层 / 账本 / rng——既隔离了实现变动，也守住确定性契约。
//!
//! ## 设计纪律
//! - 所有方法均**委托已有确定性核心**，零额外 rng、不破坏 P0/P2 复现契约与守恒律。
//! - 只读方法取 `&self`（`world.read()` 不可变借用）；推进世界经 `step(&mut self)`。
//! - 对外暴露**数据与快照**（`WorldExport` / `EconomyMetrics` / `WorldEvent`），不暴露内部层。
//!
//! ## 典型用法
//! ```no_run
//! use wanxiang_sdk::WorldModel;
//! use wanxiang_types::{WorldRules, WorldType};
//!
//! let mut rules = WorldRules::for_genre(WorldType::Xianxia);
//! let mut world = WorldModel::new(rules, 42);
//! world.step(50);                       // 推进 50 tick
//! println!("{}", world.info_json());    // 概要
//! println!("{}", world.economy_json()); // 经济标定
//! let events = world.recent_events(10); // 最近 10 条事件
//! ```

use serde::{de::Error as SerdeDeError, Deserialize, Serialize};
use wanxiang_engine::LivingCore;
use wanxiang_types::{world_rules_for_genre_id, EntityManager, EventImportance, EventType, WorldEvent, WorldRules};
use wanxiang_world::{
    calibrate_economy, devour_pass, DevourConfig, diff_world, world_export, world_report,
    EconomyMetrics, TravelExport, WorldDiff, WorldExport, WorldReport, WorldSimulator, WorldSpawner,
};

/// 题材预设（配置驱动的世界构造）：把「世界设定」从硬编码函数降级为数据。
/// 食天只是 `GenrePreset::shitian()` 一个实例——引擎对题材零硬编码。
pub mod presets;
pub use presets::{CharacterSpec, FactionSpec, GenrePreset, LocationSpec, WorldTuning};

/// 万象世界模型 —— 下游嵌入的稳定入口。
///
/// 持有一个 `LivingCore`（确定性世界核心）。所有方法委托核心，不引入新的不确定性面。
pub struct WorldModel {
    core: LivingCore,
    /// 本世界所用的题材预设（配置实例）。通用构造为 `GenrePreset::generic()`（空，无框架约束）；
    /// 题材预设世界（如食天）携带势力/角色/调参，并由 `step_preset` 据此维持框架。
    preset: GenrePreset,
    /// 保留初始构造方式，确保 fork / branch 能恢复预设、初始实体和认知开关。
    initialization: WorldInitialization,
    /// 记录推进操作，确保包含预设动力学与不推进 tick 的操作也能准确重放。
    operations: Vec<WorldOperation>,
    /// Deterministic summary of travel time, encounter and abstract supply accounting.
    travel: TravelExport,
    /// 变体重放期间不重复记录操作。
    replaying: bool,
}

/// 描述如何构造世界初始状态。该信息用于确定性重建，而不是当前世界的快照。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WorldInitialization {
    /// 默认由 seed + rules 派生初始世界。
    Seeded,
    /// 默认初始世界，并启用实例级认知层。
    SeededWithCognition {
        theory_of_mind: bool,
        counterfactual: bool,
        meme: bool,
    },
    /// 保存调用方提供的初始实体集。分支规则时固定初始条件，避免把初始人口变化混入规则效应。
    SeededEntities {
        entities: EntityManager,
        theory_of_mind: bool,
        counterfactual: bool,
        meme: bool,
    },
    /// 配置驱动的题材预设世界。
    Preset {
        preset: GenrePreset,
    },
}

/// 可重放的 WorldModel 操作。保存参数而不是只记录最终 tick。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WorldOperation {
    Step {
        ticks: u64,
    },
    PresetStep {
        ticks: u64,
        appetite_base: f64,
        devourer_ids: Vec<String>,
    },
    DevourStep {
        appetite_base: f64,
        devourer_ids: Vec<String>,
    },
    /// A validated, route-aware character movement. The operation index makes its audit event ID stable.
    MoveCharacter {
        character_id: String,
        destination_id: String,
        operation_index: u64,
    },
    /// Simulated multi-hop travel. The complete outcome is re-derived from the deterministic world.
    TravelCharacter {
        character_id: String,
        destination_id: String,
        operation_index: u64,
    },
}

/// 可检查的世界变体描述，同时保存重建初始状态和达到当前状态所需的操作序列。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldVariant {
    pub seed: u64,
    pub rules: WorldRules,
    pub tick: u64,
    pub initialization: WorldInitialization,
    pub operations: Vec<WorldOperation>,
}

/// Auditable result of one simulated journey. Costs are abstract model units, not geography.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TravelReport {
    pub character_id: String,
    pub destination_id: String,
    /// Stops actually reached; on interruption this may be a prefix of the planned route.
    pub path: Vec<String>,
    pub route_cost: f64,
    pub estimated_ticks: u64,
    pub elapsed_ticks: u64,
    /// Accumulated probability-like exposure score in [0, 1], not a calibrated real-world probability.
    pub risk_score: f64,
    pub supplies_consumed: f64,
    pub supplies_remaining: f64,
    pub encounters: Vec<String>,
    pub completed: bool,
    pub interruption_reason: Option<String>,
}

/// Read-only forecast for one character's deterministic travel under the current world state.
/// Risk is a model exposure score, and supplies are abstract simulation units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TravelEstimate {
    pub character_id: String,
    pub origin_id: String,
    pub destination_id: String,
    pub path: Vec<String>,
    pub route_cost: f64,
    pub estimated_ticks: u64,
    pub risk_score: f64,
    pub supplies_required: f64,
    pub supplies_available: f64,
    pub supplies_remaining_after: f64,
    pub supplies_shortfall: f64,
    pub can_afford: bool,
    pub predicted_encounters: Vec<String>,
}

#[derive(Clone, Debug)]
struct PlannedTravelLeg {
    from_id: String,
    to_id: String,
    elapsed_ticks: u64,
    risk: f64,
    encounter: bool,
    encounter_message: String,
    supplies: f64,
}

#[derive(Clone, Debug)]
struct BuiltTravelPlan {
    estimate: TravelEstimate,
    legs: Vec<PlannedTravelLeg>,
}

/// Stable roll in [0, 1); travel events do not consume the world's random stream.
fn travel_roll(seed: u64, character_id: &str, from_id: &str, to_id: &str, departure_tick: u64) -> f64 {
    let input = format!("{seed}|{character_id}|{from_id}|{to_id}|{departure_tick}");
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    ((hash >> 11) as f64) / ((1_u64 << 53) as f64)
}

impl WorldModel {
    /// 创建新世界（构造即生成初始世界，与 `LivingCore::new_with_seed` 一致）。
    pub fn new(rules: WorldRules, seed: u64) -> Self {
        Self::build_from_initialization(rules, seed, WorldInitialization::Seeded)
    }

    /// 在默认随机初始世界之上，叠加外部世界设定（势力/人物/地点）作为初始实体，
    /// 再经 `with_entities` 统一注册大脑 + 重算守恒账本。用于「文件读档即演化」：
    /// 用户提供的 JSON/MD 世界设定被播种进模拟器，使这些具体实体真正参与 8 层演化。
    ///
    /// 闭包 `f` 注入的实体**必须使用确定性 id**（如 `fac_spec_0` / `char_spec_0`），
    /// 以维持万象的复现契约（禁止 UUID 等不确定来源）。其余（默认人口 + 你的设定）
    /// 由 `spawn_world_seeded` 同种子确定性生成。
    pub fn new_seeded_with<F: FnOnce(&mut EntityManager)>(rules: WorldRules, seed: u64, f: F) -> Self {
        let mut ents = WorldSpawner::new().spawn_world_seeded(&rules, seed);
        f(&mut ents);
        Self::build_from_initialization(
            rules,
            seed,
            WorldInitialization::SeededEntities {
                entities: ents,
                theory_of_mind: false,
                counterfactual: false,
                meme: false,
            },
        )
    }

    /// 带安全认知层创建世界（接入层新增）：
    /// 在 `new` 基础上开启心智理论 / 反事实 / 模因进化——三者纯计算、只产描述性事件、
    /// 不改守恒量，同 seed 仍可复现（零 rng 或 rng 子流注入），守恒 drift 依旧 ≈0。
    /// 对应万象铁律「新能力默认关、开启须双 seed 逐字节一致」的运行时版：全局 layers
    /// 常量保持 false（契约测试锁定），本方法仅对该实例生效。
    pub fn new_with_cognition(rules: WorldRules, seed: u64, enable_theory_of_mind: bool, enable_counterfactual: bool, enable_meme: bool) -> Self {
        Self::build_from_initialization(
            rules,
            seed,
            WorldInitialization::SeededWithCognition {
                theory_of_mind: enable_theory_of_mind,
                counterfactual: enable_counterfactual,
                meme: enable_meme,
            },
        )
    }

    /// 带安全认知层 + 种子实体创建（接入层新增）：
    /// `new_seeded_with` 的认知层增强版——播种外部设定实体的同时开启
    /// 心智理论 / 反事实 / 模因（纯计算、只产描述性事件、不改守恒量，同 seed 可复现）。
    pub fn new_seeded_with_cognition<F: FnOnce(&mut EntityManager)>(
        rules: WorldRules,
        seed: u64,
        f: F,
        enable_theory_of_mind: bool,
        enable_counterfactual: bool,
        enable_meme: bool,
    ) -> Self {
        let mut ents = WorldSpawner::new().spawn_world_seeded(&rules, seed);
        f(&mut ents);
        Self::build_from_initialization(
            rules,
            seed,
            WorldInitialization::SeededEntities {
                entities: ents,
                theory_of_mind: enable_theory_of_mind,
                counterfactual: enable_counterfactual,
                meme: enable_meme,
            },
        )
    }

    /// 由「题材预设」构造任意确定性世界（配置驱动，题材无关）。
    ///
    /// `preset` 携带势力 / 角色 / 地点 / 调参 / 认知层 / 吞噬者等全部设定；引擎对这些具体
    /// 内容零硬编码——`genre_id` 仅用于取 `world_rules_for_genre_id` 的默认规则，调参由
    /// `preset.tuning` 覆盖。换一本小说 = 写一份 `GenrePreset`，不动引擎。
    ///
    /// 通用构造（`WorldModel::new` 等）使用的 `GenrePreset::generic()` 不含任何实体，
    /// `step_preset` 对其为 no-op，行为与通用世界一致。
    pub fn from_preset(preset: GenrePreset, seed: u64) -> Self {
        let mut rules = world_rules_for_genre_id(&preset.genre_id);
        if !preset.world_name.is_empty() {
            rules.world_name = preset.world_name.clone();
        }
        let t = &preset.tuning;
        rules.energy_density = t.energy_density;
        rules.resource_abundance = t.resource_abundance;
        rules.chaos_level = t.chaos_level;
        rules.order_level = t.order_level;
        rules.life_fertility = t.life_fertility;
        Self::build_from_initialization(
            rules,
            seed,
            WorldInitialization::Preset { preset },
        )
    }

    /// 根据保存的初始构造方式建立新实例。传入 rules 允许 branch 在不重置题材配置的情况下
    /// 修改有效运行规则。
    fn build_from_initialization(
        rules: WorldRules,
        seed: u64,
        initialization: WorldInitialization,
    ) -> Self {
        let (core, preset) = match &initialization {
            WorldInitialization::Seeded => (
                LivingCore::new_with_seed(rules, seed, false),
                GenrePreset::generic(),
            ),
            WorldInitialization::SeededWithCognition {
                theory_of_mind,
                counterfactual,
                meme,
            } => {
                let sim = WorldSimulator::new_with_seed(rules, seed)
                    .with_cognitive_layers(*theory_of_mind, *counterfactual, *meme);
                (LivingCore::with_world(sim, false), GenrePreset::generic())
            }
            WorldInitialization::SeededEntities {
                entities,
                theory_of_mind,
                counterfactual,
                meme,
            } => {
                let sim = WorldSimulator::new_with_seed(rules, seed)
                    .with_entities(entities.clone())
                    .with_cognitive_layers(*theory_of_mind, *counterfactual, *meme);
                (LivingCore::with_world(sim, false), GenrePreset::generic())
            }
            WorldInitialization::Preset { preset } => {
                let sim = WorldSimulator::new_with_seed(rules, seed)
                    .with_entities(preset.build_entities())
                    .with_cognitive_layers(
                        preset.enable_cognition.0,
                        preset.enable_cognition.1,
                        preset.enable_cognition.2,
                    );
                (LivingCore::with_world(sim, false), preset.clone())
            }
        };

        Self {
            core,
            preset,
            initialization,
            operations: Vec::new(),
            travel: TravelExport::default(),
            replaying: false,
        }
    }

    /// 食天界开局预设（第一条小说线锚定实例）：即 `from_preset(GenrePreset::shitian(), seed)`。
    /// 食天只是 `GenrePreset::shitian()` 一份配置实例——引擎对食天零硬编码（题材无关铁律实证）。
    pub fn new_shitian(seed: u64) -> Self {
        Self::from_preset(GenrePreset::shitian(), seed)
    }

    /// 构造种子。
    pub fn seed(&self) -> u64 {
        self.core.world.read().seed
    }

    /// 当前 tick。
    pub fn current_tick(&self) -> u64 {
        self.core.world.read().tick
    }

    /// 推进 `ticks` 个 tick（确定性：同 seed 同输入 → 逐帧一致）。
    pub fn step(&mut self, ticks: u64) {
        if ticks > 0 {
            self.core.run(ticks);
            if !self.replaying {
                // Coalesce adjacent simple steps. Live servers often advance one tick per
                // second; keeping one cumulative operation avoids unbounded checkpoint growth.
                match self.operations.last_mut() {
                    Some(WorldOperation::Step { ticks: previous }) => {
                        *previous = previous.saturating_add(ticks);
                    }
                    _ => self.operations.push(WorldOperation::Step { ticks }),
                }
            }
        }
    }

    /// Compute a deterministic minimum-cost route through authored location connections.
    ///
    /// Edge cost is explicit simulation metadata, not real-world time: horizontal local-space
    /// distance (or 1.0 when coordinates are absent), 0.25 × absolute elevation delta,
    /// 2.0 × destination danger (clamped to 0..1), and the destination's non-negative travel_cost
    /// (default 1.0). Equal-cost routes prefer fewer hops, then lexicographically smaller ID paths.
    /// Compute a deterministic minimum-cost route through authored location connections.
    /// Generic planning does not assume a character's ability to handle danger.
    pub fn plan_location_route(&self, from_id: &str, to_id: &str) -> Result<Vec<String>, String> {
        self.plan_location_route_with_danger_resistance(from_id, to_id, 0.0)
    }

    /// Plan for a specific living character. Higher power rank reduces the danger component only;
    /// terrain, distance, elevation and authored travel costs remain explicit.
    pub fn plan_character_route(&self, character_id: &str, destination_id: &str) -> Result<Vec<String>, String> {
        let (origin_id, alive, power_rank) = {
            let guard = self.core.world.read();
            let character = guard.entities.characters.get(character_id)
                .ok_or_else(|| format!("character '{character_id}' does not exist"))?;
            (character.location_id.clone(), character.alive, character.power_rank)
        };
        if !alive {
            return Err(format!("character '{character_id}' is not alive"));
        }
        let origin_id = origin_id
            .ok_or_else(|| format!("character '{character_id}' has no current location"))?;
        self.plan_location_route_with_danger_resistance(
            &origin_id,
            destination_id,
            Self::danger_resistance(power_rank),
        )
    }

    fn danger_resistance(power_rank: i32) -> f64 {
        let rank = power_rank.max(0) as f64;
        (rank / (rank + 20.0)).clamp(0.0, 0.75)
    }

    fn plan_location_route_with_danger_resistance(
        &self,
        from_id: &str,
        to_id: &str,
        danger_resistance: f64,
    ) -> Result<Vec<String>, String> {
        if from_id.trim().is_empty() || to_id.trim().is_empty() {
            return Err("from_id and to_id must be non-empty".to_string());
        }
        let guard = self.core.world.read();
        let locations = &guard.entities.locations;
        if !locations.contains(from_id) {
            return Err(format!("origin location '{from_id}' does not exist"));
        }
        if !locations.contains(to_id) {
            return Err(format!("destination location '{to_id}' does not exist"));
        }
        if from_id == to_id {
            return Ok(vec![from_id.to_string()]);
        }

        // Each label stores quantized cost and the full path so all tie breaks are explicit.
        let mut unvisited: std::collections::BTreeSet<String> =
            locations.ids_str().into_iter().collect();
        let mut best: std::collections::HashMap<String, (u64, Vec<String>)> =
            std::collections::HashMap::new();
        best.insert(from_id.to_string(), (0, vec![from_id.to_string()]));

        while !unvisited.is_empty() {
            let mut next: Option<(String, u64, Vec<String>)> = None;
            for id in &unvisited {
                let Some((cost, path)) = best.get(id) else { continue };
                let should_select = next.as_ref().map_or(true, |(_, best_cost, best_path)| {
                    (*cost, path.len(), path.as_slice())
                        < (*best_cost, best_path.len(), best_path.as_slice())
                });
                if should_select {
                    next = Some((id.clone(), *cost, path.clone()));
                }
            }
            let Some((current_id, current_cost, current_path)) = next else {
                break;
            };
            unvisited.remove(&current_id);
            if current_id == to_id {
                return Ok(current_path);
            }

            let Some(origin) = locations.get(&current_id) else { continue };
            let mut neighbors = origin.connected_ids.clone();
            neighbors.sort_unstable();
            neighbors.dedup();
            for neighbor_id in neighbors {
                if !unvisited.contains(&neighbor_id) {
                    continue;
                }
                let Some(destination) = locations.get(&neighbor_id) else { continue };
                let edge_cost = Self::location_edge_cost_units_with_resistance(
                    origin, destination, danger_resistance,
                );
                let candidate_cost = current_cost.saturating_add(edge_cost);
                let mut candidate_path = current_path.clone();
                candidate_path.push(neighbor_id.clone());
                let should_update = best.get(&neighbor_id).map_or(true, |(best_cost, best_path)| {
                    (candidate_cost, candidate_path.len(), candidate_path.as_slice())
                        < (*best_cost, best_path.len(), best_path.as_slice())
                });
                if should_update {
                    best.insert(neighbor_id, (candidate_cost, candidate_path));
                }
            }
        }
        Err(format!("no route connects '{from_id}' to '{to_id}'"))
    }

    fn location_edge_cost_units(
        origin: &wanxiang_types::Location,
        destination: &wanxiang_types::Location,
    ) -> u64 {
        Self::location_edge_cost_units_with_resistance(origin, destination, 0.0)
    }

    fn location_edge_cost_units_with_resistance(
        origin: &wanxiang_types::Location,
        destination: &wanxiang_types::Location,
        danger_resistance: f64,
    ) -> u64 {
        let positions = origin.position.zip(destination.position)
            .filter(|(a, b)| a.is_finite() && b.is_finite());
        let (horizontal_distance, elevation_penalty) = if let Some((a, b)) = positions {
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let horizontal = dx.hypot(dy).min(1_000_000.0).max(0.001);
            let vertical = ((a.elevation - b.elevation).abs() * 0.25).min(1_000_000.0);
            (horizontal, vertical)
        } else {
            (1.0, 0.0)
        };
        let resistance = if danger_resistance.is_finite() {
            danger_resistance.clamp(0.0, 0.75)
        } else {
            0.0
        };
        let danger_penalty = if destination.danger_level.is_finite() {
            destination.danger_level.clamp(0.0, 1.0) * 2.0 * (1.0 - resistance)
        } else {
            0.0
        };
        let authored_travel_cost = destination.travel_cost
            .filter(|cost| cost.is_finite() && *cost >= 0.0)
            .unwrap_or(1.0)
            .min(1_000_000.0);
        let terrain = destination.terrain.as_deref().unwrap_or("").to_ascii_lowercase();
        let terrain_penalty = if terrain.contains("沼") || terrain.contains("swamp") || terrain.contains("marsh") {
            1.75
        } else if terrain.contains("水") || terrain.contains("海") || terrain.contains("water") {
            1.4
        } else if terrain.contains("沙") || terrain.contains("desert") {
            1.35
        } else if terrain.contains("雪") || terrain.contains("snow") || terrain.contains("mountain") || terrain.contains("山") {
            1.1
        } else if terrain.contains("深渊") || terrain.contains("abyss") {
            2.0
        } else if terrain.contains("城") || terrain.contains("city") || terrain.contains("town") {
            0.2
        } else if terrain.contains("林") || terrain.contains("forest") {
            0.45
        } else {
            0.0
        };
        let cost = (horizontal_distance + elevation_penalty + danger_penalty
            + authored_travel_cost + terrain_penalty)
            .min(9_000_000_000.0);
        (cost * 1_000_000.0).round() as u64
    }

    /// Calculate the total model cost of a path; every hop must be an authored route edge.
    /// The value is dimensionless model cost, not seconds, kilometers, or a geographic distance.
    pub fn location_route_cost(&self, path: &[String]) -> Result<f64, String> {
        if path.is_empty() {
            return Err("path cannot be empty".to_string());
        }
        let guard = self.core.world.read();
        let locations = &guard.entities.locations;
        let mut total = 0_u64;
        for pair in path.windows(2) {
            let origin = locations.get(&pair[0])
                .ok_or_else(|| format!("route references unknown location '{}'", pair[0]))?;
            let destination = locations.get(&pair[1])
                .ok_or_else(|| format!("route references unknown location '{}'", pair[1]))?;
            if !origin.connected_ids.iter().any(|id| id == &pair[1]) {
                return Err(format!("no authored route from '{}' to '{}'", pair[0], pair[1]));
            }
            total = total.saturating_add(Self::location_edge_cost_units(origin, destination));
        }
        Ok(total as f64 / 1_000_000.0)
    }

    /// Move a character immediately along the deterministic minimum-cost authored route.
    /// Kept for callers that need legacy, no-time-advance travel semantics.
    pub fn travel_character(&mut self, character_id: &str, destination_id: &str) -> Result<Vec<String>, String> {
        let path = self.plan_character_route(character_id, destination_id)?;
        if path.len() < 2 {
            return Err(format!("character '{character_id}' is already at '{destination_id}'"));
        }
        for waypoint in path.iter().skip(1) {
            self.move_character(character_id, waypoint)?;
        }
        Ok(path)
    }

    /// Produce a read-only route/time/risk/supply forecast. This call never mutates world state.
    pub fn plan_travel_character(
        &self,
        character_id: &str,
        destination_id: &str,
    ) -> Result<TravelEstimate, String> {
        Ok(self.build_travel_plan(character_id, destination_id)?.estimate)
    }

    /// Simulate a journey: advance world ticks, charge a deterministic world travel-supply budget,
    /// apply character capability to route/risk, and emit reproducible encounter events. Invalid
    /// routes and insufficient supplies are rejected before mutation.
    pub fn simulate_travel_character(
        &mut self,
        character_id: &str,
        destination_id: &str,
    ) -> Result<TravelReport, String> {
        let operation_index = self.operations.len() as u64;
        let report = self.apply_simulated_travel(character_id, destination_id, operation_index)?;
        if !self.replaying {
            self.operations.push(WorldOperation::TravelCharacter {
                character_id: character_id.to_string(),
                destination_id: destination_id.to_string(),
                operation_index,
            });
        }
        Ok(report)
    }

    fn advance_travel_ticks(&mut self, ticks: u64) {
        // Route travel is one replayable operation. Suppress nested Step/PresetStep log entries
        // while retaining per-tick preset dynamics and frame invariants.
        let was_replaying = self.replaying;
        self.replaying = true;
        let appetite_base = self.preset.appetite_base;
        let devourer_ids = self.preset.devourer_ids.clone();
        self.step_preset(ticks, appetite_base, devourer_ids);
        self.replaying = was_replaying;
    }

    fn build_travel_plan(
        &self,
        character_id: &str,
        destination_id: &str,
    ) -> Result<BuiltTravelPlan, String> {
        let (origin_id, alive, power_rank, seed, departure_tick, character_name) = {
            let guard = self.core.world.read();
            let character = guard.entities.characters.get(character_id)
                .ok_or_else(|| format!("character '{character_id}' does not exist"))?;
            let origin_id = character.location_id.clone()
                .ok_or_else(|| format!("character '{character_id}' has no current location"))?;
            (origin_id, character.alive, character.power_rank, guard.seed, guard.tick, character.name.clone())
        };
        if !alive {
            return Err(format!("character '{character_id}' is not alive"));
        }
        let resistance = Self::danger_resistance(power_rank);
        let path = self.plan_location_route_with_danger_resistance(&origin_id, destination_id, resistance)?;
        if path.len() < 2 {
            return Err(format!("character '{character_id}' is already at '{destination_id}'"));
        }

        // Forecast the entire trip before mutation so callers can review risk and affordability.
        let mut legs: Vec<PlannedTravelLeg> = Vec::new();
        let mut planned_tick = departure_tick;
        let mut route_cost = 0.0_f64;
        let mut estimated_ticks = 0_u64;
        let mut supplies_required = 0.0_f64;
        let mut combined_risk = 0.0_f64;
        let mut predicted_encounters = Vec::new();
        {
            let guard = self.core.world.read();
            for pair in path.windows(2) {
                let origin = guard.entities.locations.get(&pair[0])
                    .ok_or_else(|| format!("route references unknown location '{}'", pair[0]))?;
                let destination = guard.entities.locations.get(&pair[1])
                    .ok_or_else(|| format!("route references unknown location '{}'", pair[1]))?;
                if !origin.connected_ids.iter().any(|id| id == &pair[1]) {
                    return Err(format!("no authored route from '{}' to '{}'", pair[0], pair[1]));
                }
                let edge_cost = Self::location_edge_cost_units_with_resistance(origin, destination, resistance) as f64 / 1_000_000.0;
                let base_ticks = ((edge_cost / 2.5).ceil() as u64).clamp(1, 250);
                let elevation_delta = origin.position.zip(destination.position)
                    .filter(|(a, b)| a.is_finite() && b.is_finite())
                    .map(|(a, b)| (a.elevation - b.elevation).abs())
                    .unwrap_or(0.0)
                    .min(500.0);
                let terrain = destination.terrain.as_deref().unwrap_or("").to_ascii_lowercase();
                let terrain_risk = if terrain.contains("沼") || terrain.contains("swamp") || terrain.contains("marsh")
                    || terrain.contains("水") || terrain.contains("海") || terrain.contains("water") {
                    0.18
                } else if terrain.contains("雪") || terrain.contains("snow") || terrain.contains("山")
                    || terrain.contains("mountain") || terrain.contains("深渊") || terrain.contains("abyss") {
                    0.12
                } else {
                    0.04
                };
                let danger = if destination.danger_level.is_finite() {
                    destination.danger_level.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let capability = (power_rank.max(0) as f64 / 100.0).clamp(0.0, 0.55);
                let risk = (0.02 + danger * 0.55 + terrain_risk + elevation_delta * 0.001)
                    .clamp(0.02, 0.90) * (1.0 - capability);
                let encounter = travel_roll(seed, character_id, &pair[0], &pair[1], planned_tick) < risk;
                let elapsed_ticks = base_ticks.saturating_add(if encounter { 2 } else { 0 });
                let supplies = (edge_cost * 0.08 + base_ticks as f64 * 0.25
                    + risk * 1.2 + if encounter { 1.0 } else { 0.0 })
                    .max(0.1);
                let encounter_message = format!(
                    "旅行遭遇：{} 从 {} 前往 {} 时遭遇危险（风险分数 {:.3}），额外耗时 2 tick",
                    character_name, pair[0], pair[1], risk
                );
                if encounter {
                    predicted_encounters.push(encounter_message.clone());
                }
                route_cost += edge_cost;
                estimated_ticks = estimated_ticks.saturating_add(elapsed_ticks);
                supplies_required += supplies;
                combined_risk = 1.0 - (1.0 - combined_risk) * (1.0 - risk);
                legs.push(PlannedTravelLeg {
                    from_id: pair[0].clone(),
                    to_id: pair[1].clone(),
                    elapsed_ticks,
                    risk,
                    encounter,
                    encounter_message,
                    supplies,
                });
                planned_tick = planned_tick.saturating_add(elapsed_ticks);
            }
        }
        let supplies_available = self.travel.supplies_remaining;
        let supplies_shortfall = (supplies_required - supplies_available).max(0.0);
        let can_afford = supplies_shortfall <= f64::EPSILON;
        Ok(BuiltTravelPlan {
            estimate: TravelEstimate {
                character_id: character_id.to_string(),
                origin_id,
                destination_id: destination_id.to_string(),
                path,
                route_cost,
                estimated_ticks,
                risk_score: combined_risk,
                supplies_required,
                supplies_available,
                supplies_remaining_after: (supplies_available - supplies_required).max(0.0),
                supplies_shortfall,
                can_afford,
                predicted_encounters,
            },
            legs,
        })
    }

    fn apply_simulated_travel(
        &mut self,
        character_id: &str,
        destination_id: &str,
        operation_index: u64,
    ) -> Result<TravelReport, String> {
        let built = self.build_travel_plan(character_id, destination_id)?;
        if !built.estimate.can_afford {
            return Err(format!(
                "insufficient travel supplies: need {:.3}, remaining {:.3}",
                built.estimate.supplies_required, built.estimate.supplies_available
            ));
        }
        let origin_id = built.estimate.origin_id.clone();
        let route_cost = built.estimate.route_cost;
        let estimated_ticks = built.estimate.estimated_ticks;
        let legs = built.legs;

        self.travel.journeys_attempted = self.travel.journeys_attempted.saturating_add(1);
        self.travel.last_route_cost = route_cost;
        self.travel.last_risk_score = 0.0;
        self.travel.last_supplies_consumed = 0.0;
        self.travel.last_encounters = 0;

        let mut actual_path = vec![origin_id.clone()];
        let mut encounters = Vec::new();
        let mut elapsed_ticks = 0_u64;
        let mut consumed = 0.0_f64;
        let mut combined_risk = 0.0_f64;
        for (hop, leg) in legs.iter().enumerate() {
            // The world continues evolving during travel; the character reaches a waypoint only
            // after this leg's simulation time has elapsed.
            self.advance_travel_ticks(leg.elapsed_ticks);
            elapsed_ticks = elapsed_ticks.saturating_add(leg.elapsed_ticks);
            consumed += leg.supplies;
            combined_risk = 1.0 - (1.0 - combined_risk) * (1.0 - leg.risk);
            self.travel.supplies_remaining = (self.travel.supplies_remaining - leg.supplies).max(0.0);
            self.travel.total_travel_ticks = self.travel.total_travel_ticks.saturating_add(leg.elapsed_ticks);
            self.travel.last_risk_score = combined_risk;
            self.travel.last_supplies_consumed = consumed;
            self.travel.last_route_cost = route_cost;

            let (alive_now, actual_location, character_name) = {
                let guard = self.core.world.read();
                guard.entities.characters.get(character_id)
                    .map(|character| (character.alive, character.location_id.clone(), character.name.clone()))
                    .ok_or_else(|| format!("character '{character_id}' disappeared during travel"))?
            };
            if !alive_now || actual_location.as_deref() != Some(leg.from_id.as_str()) {
                let reason = if !alive_now {
                    format!("{} 在途中死亡，旅行中断", character_name)
                } else {
                    format!("{} 的当前位置在旅行中发生变化，旅行中断", character_name)
                };
                let mut guard = self.core.world.write();
                let tick = guard.tick;
                let mut event = WorldEvent::new(
                    tick, EventType::WorldChange, EventImportance::Major,
                    format!("旅行中断：{}，当前位置为 {}", reason, actual_location.as_deref().unwrap_or("未知")),
                );
                event.actors.push(character_id.to_string());
                event.location = actual_location.clone();
                guard.event_engine.log(event);
                self.travel.last_encounters = encounters.len();
                let remaining = self.travel.supplies_remaining;
                return Ok(TravelReport {
                    character_id: character_id.to_string(),
                    destination_id: destination_id.to_string(),
                    path: actual_path,
                    route_cost,
                    estimated_ticks,
                    elapsed_ticks,
                    risk_score: combined_risk,
                    supplies_consumed: consumed,
                    supplies_remaining: remaining,
                    encounters,
                    completed: false,
                    interruption_reason: Some(reason),
                });
            }

            if leg.encounter {
                let message = leg.encounter_message.clone();
                let mut guard = self.core.world.write();
                let tick = guard.tick;
                let mut event = WorldEvent::new(tick, EventType::WorldChange, EventImportance::Notable, message.clone());
                event.actors.push(character_id.to_string());
                event.location = Some(leg.to_id.clone());
                guard.event_engine.log(event);
                encounters.push(message);
            }
            self.apply_character_move(
                character_id,
                &leg.to_id,
                operation_index.saturating_add(hop as u64),
            )?;
            actual_path.push(leg.to_id.clone());
        }

        self.travel.journeys_completed = self.travel.journeys_completed.saturating_add(1);
        self.travel.last_encounters = encounters.len();
        Ok(TravelReport {
            character_id: character_id.to_string(),
            destination_id: destination_id.to_string(),
            path: actual_path,
            route_cost,
            estimated_ticks,
            elapsed_ticks,
            risk_score: combined_risk,
            supplies_consumed: consumed,
            supplies_remaining: self.travel.supplies_remaining,
            encounters,
            completed: true,
            interruption_reason: None,
        })
    }

    /// Move a living character through an explicitly connected route.
    ///
    /// This action does not invent routes or advance world time. It appends a deterministic event
    /// and is preserved by checkpoint/fork/branch replay. Failed validation leaves state unchanged.
    pub fn move_character(&mut self, character_id: &str, destination_id: &str) -> Result<(), String> {
        let operation_index = self.operations.len() as u64;
        self.apply_character_move(character_id, destination_id, operation_index)?;
        if !self.replaying {
            self.operations.push(WorldOperation::MoveCharacter {
                character_id: character_id.to_string(),
                destination_id: destination_id.to_string(),
                operation_index,
            });
        }
        Ok(())
    }

    fn apply_character_move(
        &mut self,
        character_id: &str,
        destination_id: &str,
        operation_index: u64,
    ) -> Result<(), String> {
        if character_id.trim().is_empty() || destination_id.trim().is_empty() {
            return Err("character_id and destination_id must be non-empty".to_string());
        }
        let mut guard = self.core.world.write();
        let sim: &mut WorldSimulator = &mut *guard;
        let (character_name, current_location_id, alive) = sim
            .entities
            .characters
            .get(character_id)
            .map(|character| (
                character.name.clone(),
                character.location_id.clone(),
                character.alive,
            ))
            .ok_or_else(|| format!("character '{character_id}' does not exist"))?;
        if !alive {
            return Err(format!("character '{character_id}' is not alive"));
        }
        let current_location_id = current_location_id
            .ok_or_else(|| format!("character '{character_id}' has no current location"))?;
        if current_location_id == destination_id {
            return Err(format!("character '{character_id}' is already at '{destination_id}'"));
        }
        let origin = sim.entities.locations.get(&current_location_id)
            .ok_or_else(|| format!("character '{character_id}' references missing location '{current_location_id}'"))?;
        let origin_name = origin.name.clone();
        if !origin.connected_ids.iter().any(|id| id == destination_id) {
            return Err(format!("no authored route from '{current_location_id}' to '{destination_id}'"));
        }
        let destination_name = sim.entities.locations.get(destination_id)
            .map(|location| location.name.clone())
            .ok_or_else(|| format!("destination location '{destination_id}' does not exist"))?;

        sim.entities.characters.get_mut(character_id)
            .expect("character was validated above")
            .location_id = Some(destination_id.to_string());
        let description = format!(
            "角色 {character_name} 沿路线从 {origin_name} 移动到 {destination_name}（操作序号 {operation_index}）"
        );
        let mut event = WorldEvent::new(sim.tick, EventType::WorldChange, EventImportance::Notable, description);
        event.actors.push(character_id.to_string());
        event.location = Some(destination_id.to_string());
        sim.event_engine.log(event);
        Ok(())
    }

    /// 对当前世界施加一次「吞噬」pass（题材无关的 Devour 动力学）。
    /// 不推进 tick：仅由 `devourer_ids` 指定角色吞噬各自最弱他者——
    /// 「吃=能量转移」经守恒账本路由（drift≈0），「过食=守恒漂移反噬」凭空注入（drift>0）。
    /// 返回本轮产生的吞噬 / 反噬事件。适用于任意世界（通用吞噬接口）。
    pub fn devour_step(&mut self, appetite_base: f64, devourer_ids: Vec<String>) -> Vec<WorldEvent> {
        let cfg = DevourConfig {
            appetite_base,
            digestion_cap: f64::INFINITY,
        };
        let mut guard = self.core.world.write();
        let sim: &mut WorldSimulator = &mut *guard;
        let t = sim.tick;
        let evs = devour_pass(&mut sim.entities, &mut sim.conservation, &cfg, t, &devourer_ids);
        for e in &evs {
            sim.event_engine.log(e.clone());
        }
        if !self.replaying {
            self.operations.push(WorldOperation::DevourStep {
                appetite_base,
                devourer_ids,
            });
        }
        evs
    }

    /// 通用「题材预设」推进：每 tick = `WorldSimulator::tick()`（事件自动入 log）+ 按预设接入动力学。
    ///
    /// - 吞噬：`devourer_ids` 非空（或预设 `devourer_ids` 非空）时每 tick 调用 `devour_pass`
    ///   （题材无关 `GenreModule::Devour` 动力学：吃=能量转移 drift≈0，过食=守恒漂移反噬 drift>0）；
    ///   为空则不接入吞噬——通用世界 / 非吞噬题材不受影响。
    /// - 框架：每 tick 调用 `preset.enforce_frame` 维持势力/角色框架（通用空预设为 no-op）。
    ///
    /// 确定性：同 `(preset, seed, ticks, appetite, devourer_ids)` → 逐帧一致。
    pub fn step_preset(
        &mut self,
        ticks: u64,
        appetite_base: f64,
        devourer_ids: Vec<String>,
    ) -> Vec<WorldEvent> {
        let cfg = DevourConfig {
            appetite_base,
            digestion_cap: f64::INFINITY,
        };
        let devs: Vec<String> = if devourer_ids.is_empty() {
            self.preset.devourer_ids.clone()
        } else {
            devourer_ids
        };
        let do_devour = !devs.is_empty();
        let preset = &self.preset;
        let mut all_devour = Vec::new();
        for _ in 0..ticks {
            let dev = {
                let mut guard = self.core.world.write();
                let sim: &mut WorldSimulator = &mut *guard;
                let _tick_events = sim.tick(); // tick 内部已统一 log 事件
                let t = sim.tick; // tick() 自增后的当前 tick
                let d = if do_devour {
                    devour_pass(&mut sim.entities, &mut sim.conservation, &cfg, t, &devs)
                } else {
                    Vec::new()
                };
                for e in &d {
                    sim.event_engine.log(e.clone());
                }
                preset.enforce_frame(&mut sim.entities);
                d
            };
            all_devour.extend(dev);
        }
        if !self.replaying {
            self.operations.push(WorldOperation::PresetStep {
                ticks,
                appetite_base,
                devourer_ids: devs,
            });
        }
        all_devour
    }

    /// 食天界专用推进：即 `step_preset`（食天预设提供 `devourer_ids=["char_tao"]` 与三足框架）。
    /// 保留为语义化薄封装，供应用端 / 旧代码沿用。
    pub fn step_shitian(
        &mut self,
        ticks: u64,
        appetite_base: f64,
        devourer_ids: Vec<String>,
    ) -> Vec<WorldEvent> {
        self.step_preset(ticks, appetite_base, devourer_ids)
    }

    /// 存活角色数。
    pub fn character_count(&self) -> usize {
        self.core.world.read().entities.alive_count()
    }

    /// 角色心智快照（方向5：角色心智一致性）。
    /// 按名字查角色，返回其真实心智状态 JSON：
    /// emotion（PAD+勇气/恐惧/压力）、personality（Big Five）、goals、relationships（对他人看法）、
    /// age/power_rank/alive/faction_role。纯只读查询，不改变世界状态（确定性不受影响）。
    pub fn character_mind(&self, name: &str) -> Option<String> {
        let sim = self.core.world.read();
        let ch = sim.entities.characters.iter().into_iter().find(|(_, c)| c.name == name).map(|(_, c)| c)?;
        Some(
            serde_json::json!({
                "name": ch.name,
                "gender": format!("{:?}", ch.gender),
                "age": ch.age,
                "power_rank": ch.power_rank,
                "alive": ch.alive,
                "faction_role": ch.faction_role,
                "emotion": ch.emotion,
                "personality": ch.personality,
                "goals": ch.goals,
                "relationships": ch.relationships,
            })
            .to_string(),
        )
    }

    /// 全部角色名（心智 tab 的角色下拉用，字典序保证确定性）。
    pub fn character_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .core
            .world
            .read()
            .entities
            .characters
            .iter()
            .into_iter()
            .map(|(_, c)| c.name.clone())
            .collect();
        names.sort_unstable();
        names
    }

    /// 势力数。
    pub fn faction_count(&self) -> usize {
        self.core.world.read().entities.factions.len()
    }

    /// 事件总数（`event_log` 长度）。
    pub fn event_count(&self) -> usize {
        self.core.world.read().event_engine.event_log.len()
    }

    /// 守恒漂移（≈0 = 各层已良好经守恒律路由）。
    pub fn conservation_drift(&self) -> f64 {
        self.core.world.read().conservation.drift()
    }

    /// 世界是否守恒。
    pub fn is_conserved(&self) -> bool {
        let w = self.core.world.read();
        w.conservation.is_conserved(1e-4)
    }

    /// 最近 `n` 条事件（按 tick 顺序，取末尾 n 条）。
    pub fn recent_events(&self, n: usize) -> Vec<WorldEvent> {
        let w = self.core.world.read();
        let log = &w.event_engine.event_log;
        let start = log.len().saturating_sub(n);
        log[start..].to_vec()
    }

    /// 完整世界快照（`WorldExport`，可序列化为 JSON）。
    pub fn snapshot(&self) -> WorldExport {
        let w = self.core.world.read();
        let mut snapshot = world_export(&w);
        snapshot.travel = self.travel.clone();
        snapshot
    }

    /// 完整世界快照 JSON。
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.snapshot()).unwrap_or_else(|_| "{}".into())
    }

    /// 世界终局报告（阵营排名 + 命名议题热点 + 重大事件计数，吸收旧版产出视图）。
    /// 纯只读、确定性，与其他 SDK 方法同源零新增 rng 面。
    pub fn report(&self) -> WorldReport {
        let w = self.core.world.read();
        world_report(&w)
    }

    /// 世界终局报告 JSON。
    pub fn report_json(&self) -> String {
        serde_json::to_string(&self.report()).unwrap_or_else(|_| "{}".into())
    }

    /// 经济标定快照。
    pub fn economy(&self) -> EconomyMetrics {
        let w = self.core.world.read();
        calibrate_economy(&w)
    }

    /// 经济标定 JSON。
    pub fn economy_json(&self) -> String {
        serde_json::to_string(&self.economy()).unwrap_or_else(|_| "{}".into())
    }

    /// 概要信息 JSON（seed / tick / 计数 / 守恒 / 混沌边缘）—— 游戏后端最常用的轻量查询。
    pub fn info_json(&self) -> String {
        let w = self.core.world.read();
        serde_json::json!({
            "seed": w.seed,
            "tick": w.tick,
            "day": w.day,
            "characters": w.entities.alive_count(),
            "factions": w.entities.factions.len(),
            "events": w.event_engine.event_log.len(),
            "conservation_drift": w.conservation.drift(),
            "conserved": w.conservation.is_conserved(1e-4),
            "chaos_edge": serde_json::to_value(w.chaos_edge_report()).unwrap_or(serde_json::Value::Null),
        })
        .to_string()
    }

    // ───────────────────────── 分支 / 对比（护城河：可对比 diff/fork）─────────────────────────

    /// 由变体句柄重建世界：恢复原始构造方式，然后重放每一个公开操作。
    /// 这比只重放 tick 更完整，因为预设推进、吞噬 pass 和实例级认知开关也会影响结果。
    fn from_variant(v: &WorldVariant) -> WorldModel {
        Self::try_from_variant(v).expect("internal WorldModel operation history must replay successfully")
    }

    fn try_from_variant(v: &WorldVariant) -> Result<WorldModel, String> {
        let mut model = Self::build_from_initialization(
            v.rules.clone(),
            v.seed,
            v.initialization.clone(),
        );
        model.replaying = true;
        for (operation_position, op) in v.operations.iter().enumerate() {
            match op {
                WorldOperation::Step { ticks } => model.step(*ticks),
                WorldOperation::PresetStep { ticks, appetite_base, devourer_ids } => {
                    model.step_preset(*ticks, *appetite_base, devourer_ids.clone());
                }
                WorldOperation::DevourStep { appetite_base, devourer_ids } => {
                    model.devour_step(*appetite_base, devourer_ids.clone());
                }
                WorldOperation::MoveCharacter { character_id, destination_id, operation_index } => {
                    if *operation_index != operation_position as u64 {
                        return Err(format!("movement operation index mismatch at operation {operation_position}"));
                    }
                    model.apply_character_move(character_id, destination_id, *operation_index)?;
                }
                WorldOperation::TravelCharacter { character_id, destination_id, operation_index } => {
                    if *operation_index != operation_position as u64 {
                        return Err(format!("travel operation index mismatch at operation {operation_position}"));
                    }
                    model.apply_simulated_travel(character_id, destination_id, *operation_index)?;
                }
            }
        }
        model.replaying = false;
        model.operations = v.operations.clone();
        let replayed_tick = model.current_tick();
        if replayed_tick != v.tick {
            return Err(format!("checkpoint tick mismatch: metadata says {}, replay reached {replayed_tick}", v.tick));
        }
        Ok(model)
    }

    /// 捕获当前世界的完整重建配方（seed、有效规则、初始构造方式与操作序列）。
    pub fn checkpoint(&self) -> WorldVariant {
        let w = self.core.world.read();
        WorldVariant {
            seed: w.seed,
            rules: w.rules.rules.clone(),
            tick: w.tick,
            initialization: self.initialization.clone(),
            operations: self.operations.clone(),
        }
    }

    /// Serialize the deterministic reconstruction recipe for durable application checkpoints.
    /// This stores initialization and public operations, not an opaque in-memory runtime image.
    pub fn checkpoint_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.checkpoint())
    }

    /// Restore a checkpoint by reconstructing the original world and replaying its operations.
    /// The caller should fail closed if the checkpoint is corrupt or its schema is incompatible.
    pub fn from_checkpoint_json(raw: &str) -> Result<Self, serde_json::Error> {
        let variant: WorldVariant = serde_json::from_str(raw)?;
        Self::try_from_variant(&variant).map_err(SerdeDeError::custom)
    }

    /// 分叉：返回一个**独立**的 `WorldModel`，其当前状态与自身逐字节一致，
    /// 后续推进互不影响。典型用途——在不破坏原世界的前提下试跑一条演化路径。
    pub fn fork(&self) -> WorldModel {
        WorldModel::from_variant(&self.checkpoint())
    }

    /// 分支：在分叉基础上施加规则扰动（闭包 `f` 修改 `WorldRules`），返回一个
    /// 轨迹从此处**发散**的变体。用于隔离「某条规则改动 → 世界如何不同」的因果效应。
    ///
    /// 例：把能量密度拉满 `m.branch(|r| r.energy_density = 0.95)`，再与 `m.fork()` 对比。
    pub fn branch<F: FnOnce(&mut WorldRules)>(&self, f: F) -> WorldModel {
        let mut v = self.checkpoint();
        f(&mut v.rules);
        WorldModel::from_variant(&v)
    }

    /// 分支（换种子）：仅改变构造种子，其余（规则、当前 tick）不变。
    /// 用于演示「同规则不同种子」的蝴蝶效应分歧（spawn 即不同 → 早期事件流分歧）。
    pub fn branch_with_seed(&self, seed: u64) -> WorldModel {
        let mut v = self.checkpoint();
        v.seed = seed;
        WorldModel::from_variant(&v)
    }

    /// 对比：对两个世界状态做结构化 diff（守恒四池 Δ / 实体计数 Δ / 首分歧 tick / 规则差异）。
    /// 纯函数、零 rng、只读——绝不修改任一世界，绝不抽取 rng（复用 P4-b `diff_world`）。
    pub fn compare(&self, other: &WorldModel) -> WorldDiff {
        let a = self.core.world.read();
        let b = other.core.world.read();
        diff_world(&a, &b)
    }

    /// 对比结果 JSON（结构化，可直接给前端/脚本消费）。
    pub fn diff_json(&self, other: &WorldModel) -> String {
        let d = self.compare(other);
        let report = serde_json::json!({
            "a": {
                "seed": d.a.seed,
                "tick": d.a.tick,
                "world_type": d.a.world_type,
                "name": d.a.name,
            },
            "b": {
                "seed": d.b.seed,
                "tick": d.b.tick,
                "world_type": d.b.world_type,
                "name": d.b.name,
            },
            "rules_differ": d.rules_differ,
            "conservation_delta": d.conservation_delta,
            "total_delta": d.total_delta,
            "drift": d.drift,
            "conserved": d.conserved,
            "count_delta": d.count_delta,
            "event_count": d.event_count,
            "first_divergence_tick": d.first_divergence_tick,
            "first_divergence_events": d.first_divergence_events,
            "summary": d.summary,
        });
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
    }
}

// ───────────────────────── 题材预设（配置驱动）─────────────────────────
// 食天等「世界设定」已下沉为数据：`GenrePreset`（见 `presets.rs`）。
// `from_preset` / `step_preset` 据此构造与推进任意确定性世界，引擎对题材零硬编码。
// 新增题材 = 在 `presets.rs` 的 `GenrePreset::registry()` 追加一个实例，无需改动引擎。

// ───────────────────────── 测试 ─────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use wanxiang_types::{Character, Faction, Gender, WorldType, WorldRules};

    #[test]
    fn sdk_worldmodel_deterministic() {
        let mk = || WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        let mut a = mk();
        let mut b = mk();
        a.step(40);
        b.step(40);
        assert_eq!(a.current_tick(), b.current_tick());
        assert_eq!(a.event_count(), b.event_count());
        assert_eq!(a.snapshot_json(), b.snapshot_json(), "同 seed 快照必须逐字节一致");
        assert_eq!(a.economy_json(), b.economy_json(), "同 seed 经济标定必须一致");
        assert_eq!(a.report_json(), b.report_json(), "同 seed 终局报告必须逐字节一致");
    }

    #[test]
    fn sdk_step_advances_and_keeps_conserved() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Scifi), 7);
        assert_eq!(m.current_tick(), 0);
        m.step(30);
        assert_eq!(m.current_tick(), 30);
        assert!(m.is_conserved(), "推进后世界仍须守恒");
        assert!(m.conservation_drift().abs() < 1e-4);
    }

    #[test]
    fn sdk_info_json_has_fields() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Urban), 99);
        m.step(10);
        let info = m.info_json();
        assert!(info.contains("\"seed\":99"));
        assert!(info.contains("\"tick\":10"));
        assert!(info.contains("\"conserved\":true"));
    }

    #[test]
    fn sdk_recent_events_bounds() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 5);
        m.step(20);
        let n = m.event_count();
        let recent = m.recent_events(5);
        assert!(recent.len() <= 5);
        assert!(recent.len() <= n);
    }

    #[test]
    fn sdk_checkpoint_round_trip_restores_deterministic_world_and_future() {
        let mut original = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 2401);
        original.step(17);
        original.step(9);
        let wire = original.checkpoint_json().expect("checkpoint serializes");
        let mut restored = WorldModel::from_checkpoint_json(&wire).expect("checkpoint restores");
        assert_eq!(restored.snapshot_json(), original.snapshot_json());
        assert_eq!(restored.current_tick(), original.current_tick());
        original.step(11);
        restored.step(11);
        assert_eq!(restored.snapshot_json(), original.snapshot_json());
        let changed = restored.branch(|r| r.energy_density = 0.91);
        assert_eq!(changed.current_tick(), restored.current_tick());
    }

    #[test]
    fn sdk_fork_preserves_instance_cognition_flags() {
        let build = || {
            WorldModel::new_with_cognition(
                WorldRules::for_genre(WorldType::Xianxia),
                2026,
                true,
                true,
                true,
            )
        };
        let mut original = build();
        original.step(35);
        let mut fork = original.fork();
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
        original.step(5);
        fork.step(5);
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
    }

    #[test]
    fn sdk_fork_preserves_custom_initial_entities() {
        let build = || {
            WorldModel::new_seeded_with(
                WorldRules::for_genre(WorldType::Xianxia),
                42,
                |entities| {
                    let mut faction = Faction::new("Fork 保留测试宗门".to_string());
                    faction.id = "fac_fork_preserve".to_string();
                    entities.add_faction(faction);
                },
            )
        };
        let mut original = build();
        original.step(12);
        let mut fork = original.fork();
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
        original.step(3);
        fork.step(3);
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
        assert!(fork.snapshot_json().contains("fac_fork_preserve"));
    }

    #[test]
    fn sdk_fork_preserves_preset_and_mixed_operation_history() {
        let mut original = WorldModel::new_shitian(20260809);
        original.step_shitian(12, 12.0, vec!["char_tao".to_string()]);
        original.devour_step(7.0, vec!["char_tao".to_string()]);
        original.step(3);
        let mut fork = original.fork();
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
        original.step_shitian(2, 12.0, vec![]);
        fork.step_shitian(2, 12.0, vec![]);
        assert_eq!(fork.snapshot_json(), original.snapshot_json());
    }

    #[test]
    fn sdk_branch_preserves_custom_initial_entities_and_replays_operations() {
        let build = || {
            WorldModel::new_seeded_with(
                WorldRules::for_genre(WorldType::Xianxia),
                42,
                |entities| {
                    let mut faction = Faction::new("Branch 保留测试宗门".to_string());
                    faction.id = "fac_branch_preserve".to_string();
                    entities.add_faction(faction);
                },
            )
        };

        let mut original = build();
        original.step(8);
        // This operation does not advance the tick; branch replay must still retain it.
        original.devour_step(0.0, vec![]);

        let mut branch = original.branch(|rules| {
            rules.energy_density = 0.95;
        });
        assert_eq!(branch.current_tick(), original.current_tick());
        assert!(branch.snapshot_json().contains("fac_branch_preserve"));
        assert!(branch.compare(&original).rules_differ);

        let mut rebuilt = branch.fork();
        assert_eq!(rebuilt.snapshot_json(), branch.snapshot_json());
        branch.step(2);
        rebuilt.step(2);
        assert_eq!(rebuilt.snapshot_json(), branch.snapshot_json());
    }

    #[test]
    fn sdk_branch_preserves_preset_and_mixed_operation_history() {
        let mut original = WorldModel::new_shitian(20260809);
        original.step_shitian(8, 12.0, vec!["char_tao".to_string()]);
        original.devour_step(7.0, vec!["char_tao".to_string()]);
        original.step(2);

        let branch = original.branch(|rules| {
            rules.energy_density = 0.95;
        });
        assert_eq!(branch.current_tick(), original.current_tick());

        let rebuilt = branch.fork();
        assert_eq!(rebuilt.snapshot_json(), branch.snapshot_json());
        assert!(branch.snapshot_json().contains("fac_tunling"));
        assert!(branch.snapshot_json().contains("fac_shimeng"));
        assert!(branch.snapshot_json().contains("fac_mingge"));
    }

    // ── T2-A：fork / branch / compare 护城河 API ──

    #[test]
    fn sdk_fork_is_byte_identical() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(30);
        let f = m.fork();
        assert_eq!(f.current_tick(), m.current_tick(), "fork 同 tick");
        assert_eq!(
            f.snapshot_json(),
            m.snapshot_json(),
            "fork 必须逐字节一致（独立副本、同状态）"
        );
        assert_eq!(f.economy_json(), m.economy_json(), "fork 经济标定一致");
    }

    #[test]
    fn sdk_branch_with_seed_diverges() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(20);
        let v = m.branch_with_seed(7);
        let d = m.compare(&v);
        assert!(!d.rules_differ, "仅改 seed → 规则应相同");
        assert!(
            d.first_divergence_tick.is_some(),
            "不同 seed 应在早期分歧（蝴蝶效应）"
        );
        assert_ne!(d.event_count.0, d.event_count.1, "事件流长度应不同");
    }

    #[test]
    fn sdk_branch_rule_override_reports_difference() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(20);
        let v = m.branch(|r| {
            r.energy_density = 0.95;
            r.chaos_level = 0.1;
        });
        let d = m.compare(&v);
        assert!(d.rules_differ, "改规则参数 → 规则应判为不同");
        assert!(!d.is_identical(), "规则不同 → 不应判定为完全等同");
    }

    #[test]
    fn sdk_branch_then_diff_is_deterministic() {
        // 同一次 branch 两次应得到逐字节一致的世界（确定性）。
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Scifi), 11);
        m.step(25);
        let a = m.branch(|r| r.life_fertility = 0.9);
        let b = m.branch(|r| r.life_fertility = 0.9);
        assert_eq!(
            a.snapshot_json(),
            b.snapshot_json(),
            "相同 branch 闭包 → 逐字节一致"
        );
    }

    #[test]
    fn sdk_diff_json_parses_and_reports() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(15);
        let v = m.branch_with_seed(99);
        let j = m.diff_json(&v);
        let val: serde_json::Value = serde_json::from_str(&j).expect("diff_json 必须合法 JSON");
        assert_eq!(val["rules_differ"], serde_json::json!(false));
        assert!(
            val["first_divergence_tick"].is_number() || val["first_divergence_tick"].is_null()
        );
        assert!(val["summary"].is_array());
    }

    // ── 文件读档即演化：new_seeded_with 播种 + 确定性 ──

    #[test]
    fn sdk_new_seeded_with_injects_and_stays_conserved() {
        let build = || {
            WorldModel::new_seeded_with(WorldRules::for_genre(WorldType::Xianxia), 42, |em| {
                let mut f = Faction::new("测试宗门".to_string());
                f.id = "fac_spec_0".to_string();
                f.resources.insert("灵石".to_string(), 1000.0);
                em.add_faction(f);
                let mut c = Character::new("测试人物".to_string(), Gender::Male);
                c.id = "char_spec_0".to_string();
                c.faction_id = Some("fac_spec_0".to_string());
                c.death_protection_days = 30;
                em.add_character(c);
            })
        };
        let mut a = build();
        a.step(20);
        assert!(a.faction_count() >= 1, "播种的势力应存在");
        assert!(a.character_count() >= 1, "播种的角色应存在");
        assert!(a.is_conserved(), "播种后世界仍须守恒");

        // 同 seed + 同设定 → 逐字节一致（复现契约不被破坏）
        let mut b = build();
        b.step(20);
        assert_eq!(
            a.snapshot_json(),
            b.snapshot_json(),
            "同 seed + 同设定 必须逐字节一致"
        );
    }

    #[test]
    fn sdk_new_seeded_with_empty_spec_still_works() {
        // 空闭包：等价于默认 spawn，仍应可构造并推进
        let mut m = WorldModel::new_seeded_with(WorldRules::for_genre(WorldType::Scifi), 7, |_em| {});
        m.step(10);
        assert!(m.current_tick() == 10);
        assert!(m.is_conserved());
    }

    #[test]
    fn sdk_new_with_cognition_deterministic_and_conserved() {
        // 08-03：开启安全认知层（心智理论/反事实/模因）后——
        // ① 双 seed 逐字节一致（复现契约）② 守恒 drift≈0 ③ 认知层确实产出事件。
        let build = || {
            WorldModel::new_with_cognition(
                WorldRules::for_genre(WorldType::Xianxia),
                2026,
                true, // theory_of_mind
                true, // counterfactual
                true, // meme
            )
        };
        let mut a = build();
        a.step(60);
        assert!(a.is_conserved(), "开启认知层后世界仍须守恒（事件-only，不改守恒量）");
        assert!(a.conservation_drift().abs() < 1e-4, "守恒 drift 必须≈0");

        // 双 seed 逐字节一致
        let mut b = build();
        b.step(60);
        assert_eq!(a.snapshot_json(), b.snapshot_json(), "同 seed 开启认知层必须逐字节一致");

        // 认知事件确实出现（心智理论 / 反事实 / 模因的描述性产出）
        let evs = a.recent_events(200);
        let desc = evs.iter().map(|e| e.description.as_str()).collect::<Vec<_>>().join("\n");
        let has_cognition = desc.contains("他者") || desc.contains("反事实") || desc.contains("认为") || desc.contains("猜测")
            || desc.contains("模因") || desc.contains("观念") || desc.contains("传播");
        assert!(has_cognition, "开启认知层后事件流应含心智/反事实/模因类描述，实际:\n{}", &desc[..desc.len().min(500)]);
    }

    #[test]
    fn sdk_new_with_cognition_off_by_default_unchanged() {
        // 未显式开启时，new() 的认知层必须保持关闭（与历史行为一致）
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(30);
        assert!(m.is_conserved());
        let desc = m.recent_events(100).iter().map(|e| e.description.as_str()).collect::<Vec<_>>().join("\n");
        assert!(!desc.contains("他者") && !desc.contains("反事实"), "默认 new() 不应产出认知层事件");
    }

    #[test]
    fn sdk_character_mind_deterministic_and_readonly() {
        // 方向5：心智快照确定性——双 seed 相同 → 同名角色的心智 JSON 逐字节一致
        let mk = || {
            let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
            m.step(20);
            m
        };
        let a = mk();
        let b = mk();
        let names_a = a.character_names();
        assert!(!names_a.is_empty(), "演化世界应有角色");
        let name = names_a[0].clone();
        let mind_a = a.character_mind(&name).expect("角色应有心智");
        let mind_b = b.character_mind(&name).expect("角色应有心智");
        assert_eq!(mind_a, mind_b, "同 seed 心智快照必须逐字节一致");
        let parsed: serde_json::Value = serde_json::from_str(&mind_a).unwrap();
        assert!(parsed.get("emotion").is_some(), "心智应含 emotion");
        assert!(parsed.get("personality").is_some(), "心智应含 personality");
        assert!(parsed.get("goals").is_some(), "心智应含 goals");
        assert!(parsed.get("relationships").is_some(), "心智应含 relationships");
        // 只读：查询后世界快照不变
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(20);
        let before = m.snapshot_json();
        let _ = m.character_mind(&name);
        let _ = m.character_names();
        assert_eq!(before, m.snapshot_json(), "心智查询不得改变世界状态");
    }

    #[test]
    fn sdk_character_mind_unknown_name_returns_none() {
        let mut m = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
        m.step(10);
        assert!(m.character_mind("不存在的角色XYZ").is_none());
    }
}

#[cfg(test)]
mod shitian_integration_tests {
    use super::*;
    use wanxiang_types::Gender;

    #[test]
    fn shitian_preset_persists_three_factions_and_devours() {
        let mut w = WorldModel::new_shitian(20260809);
        let dev = w.step_shitian(120, 12.0, vec!["char_tao".to_string()]);

        // 三势力全程并存，无泛化势力残留
        let sim = w.core.world.read();
        let fac_ids: Vec<String> = sim.entities.factions.ids_str();
        assert!(fac_ids.contains(&"fac_tunling".to_string()), "吞灵宗应在");
        assert!(fac_ids.contains(&"fac_shimeng".to_string()), "散修食盟应在");
        assert!(fac_ids.contains(&"fac_mingge".to_string()), "命阁应在");
        assert_eq!(fac_ids.len(), 3, "不应有泛化势力残留，实际: {:?}", fac_ids);

        // 命名角色全程存活且归位
        for (id, fac) in [
            ("char_tao", "fac_shimeng"),
            ("char_shikong", "fac_tunling"),
            ("char_baiwei", "fac_shimeng"),
            ("char_siming", "fac_mingge"),
        ] {
            let c = sim.entities.characters.get(id).expect("命名角色应在");
            assert!(c.alive, "{} 应存活", id);
            assert_eq!(c.faction_id.as_deref(), Some(fac), "{} 应归位 {}", id, fac);
        }

        // 吞噬动力学确实产出事件
        assert!(
            dev.iter().any(|e| e.description.contains("吞噬")),
            "应产出吞噬/反噬事件"
        );
        // 终态守恒（实验框架每 tick 复位三势力资源属外部扰动，容忍小漂移）
        assert!(
            sim.conservation.is_conserved(1e-1) || sim.conservation.drift().abs() < 50.0,
            "终态守恒漂移应受控，实际 {}",
            sim.conservation.drift()
        );
    }

    #[test]
    fn shitian_preset_deterministic() {
        let mk = || {
            let mut w = WorldModel::new_shitian(20260809);
            w.step_shitian(60, 12.0, vec!["char_tao".to_string()]);
            w
        };
        let a = mk();
        let b = mk();
        assert_eq!(a.snapshot_json(), b.snapshot_json(), "食天预设必须确定性可复现");
    }

    #[test]
    fn preset_driven_arbitrary_world_works() {
        // 「换一本小说」实证：一份全新的双势力预设（非食天），不碰引擎即可构造 + 推进 + 确定性。
        let preset = GenrePreset {
            genre_id: "demo".to_string(),
            title: "演示界".to_string(),
            world_name: "演示界".to_string(),
            locations: vec![LocationSpec {
                id: "l1".into(),
                name: "中央地".into(),
                environment: "平和之地".into(),
                resource_richness: 0.7,
                danger_level: 0.2,
                connected_ids: vec![],
                position: None,
                terrain: None,
                travel_cost: None,
            }],
            factions: vec![
                FactionSpec {
                    id: "fa".into(),
                    name: "甲盟".into(),
                    territory_ids: vec!["l1".into()],
                    resources: res("灵石", 1000.0),
                },
                FactionSpec {
                    id: "fb".into(),
                    name: "乙盟".into(),
                    territory_ids: vec!["l1".into()],
                    resources: res("灵石", 1000.0),
                },
            ],
            characters: vec![
                CharacterSpec {
                    id: "ca".into(),
                    name: "阿甲".into(),
                    faction_id: "fa".into(),
                    location_id: Some("l1".into()),
                    gender: Gender::Male,
                    power_value: 100.0,
                    power_rank: 1,
                },
                CharacterSpec {
                    id: "cb".into(),
                    name: "阿乙".into(),
                    faction_id: "fb".into(),
                    location_id: Some("l1".into()),
                    gender: Gender::Female,
                    power_value: 100.0,
                    power_rank: 1,
                },
            ],
            modules: vec![],
            devourer_ids: vec![],
            appetite_base: 0.0,
            enable_cognition: (false, false, false),
            tuning: WorldTuning::default(),
        };

        let run = || {
            let mut w = WorldModel::from_preset(preset.clone(), 7);
            w.step_preset(80, 0.0, vec![]);
            // 框架保持：两势力全程并存，无泛化势力残留
            let sim = w.core.world.read();
            let fac_ids: Vec<String> = sim.entities.factions.ids_str();
            assert!(fac_ids.contains(&"fa".to_string()), "甲盟应存在");
            assert!(fac_ids.contains(&"fb".to_string()), "乙盟应存在");
            assert_eq!(fac_ids.len(), 2, "不应有泛化势力残留，实际: {:?}", fac_ids);
            w.recent_events(usize::MAX)
                .iter()
                .map(|e| (e.tick, e.description.clone()))
                .collect::<Vec<_>>()
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "同 preset + seed 应逐条一致（确定性可复现）");
    }
}

/// 测试辅助：构造单键资源表。
#[cfg(test)]
fn res(key: &str, amount: f64) -> std::collections::HashMap<String, f64> {
    let mut m = std::collections::HashMap::new();
    m.insert(key.to_string(), amount);
    m
}


#[cfg(test)]
mod long_horizon_acceptance_tests {
    use super::{GenrePreset, WorldModel};
    use wanxiang_types::{WorldRules, WorldType};

    /// A fixed-seed replay contract exercised beyond the short demo horizon.
    #[test]
    fn long_horizon_replay_is_deterministic_and_rule_sensitive() {
        let rules = WorldRules::for_genre(WorldType::Xianxia);
        let mut segmented = WorldModel::new(rules.clone(), 1_000_042);
        let mut continuous = WorldModel::new(rules, 1_000_042);

        segmented.step(400);
        segmented.step(600);
        continuous.step(1_000);

        assert_eq!(segmented.current_tick(), 1_000);
        assert_eq!(continuous.current_tick(), 1_000);
        assert_eq!(
            segmented.snapshot_json(),
            continuous.snapshot_json(),
            "same seed and equivalent operation history must replay identically"
        );
        assert!(segmented.is_conserved(), "baseline ledger must remain conserved");
        assert!(
            segmented.conservation_drift().abs() < 1e-4,
            "baseline ledger drift should remain near zero"
        );

        let altered = segmented.branch(|rules| {
            rules.energy_density = if rules.energy_density < 0.9 {
                rules.energy_density + 0.1
            } else {
                rules.energy_density - 0.1
            };
        });
        assert_eq!(altered.current_tick(), 1_000);
        assert!(altered.is_conserved(), "altered trajectory ledger must remain conserved");
        let diff: serde_json::Value = serde_json::from_str(&segmented.diff_json(&altered))
            .expect("long-horizon rule diff should be valid JSON");
        assert_eq!(diff["rules_differ"], true);
        assert_ne!(
            diff["event_count"][0],
            diff["event_count"][1],
            "changed rules should alter simulated events, not only snapshot metadata"
        );
        assert!(
            diff["first_divergence_tick"].is_number(),
            "the SDK should identify the first divergent tick"
        );
    }
}


#[cfg(test)]
mod character_movement_tests {
    use super::WorldModel;
    use wanxiang_types::{WorldRules, WorldType};

    #[test]
    fn route_move_updates_resident_event_and_replay() {
        let mut world = WorldModel::new_shitian(20261017);
        let initial = world.snapshot();
        let tao = initial.entities.iter().find(|entity| entity.id == "char_tao").expect("preset character");
        assert_eq!(tao.location_id.as_deref(), Some("loc_city"));

        world.move_character("char_tao", "loc_abyss").expect("directly connected route");
        let moved = world.snapshot();
        let tao_after = moved.entities.iter().find(|entity| entity.id == "char_tao").expect("moved character");
        assert_eq!(tao_after.location_id.as_deref(), Some("loc_abyss"));
        assert!(moved.events.iter().any(|event| {
            event.description.contains("移动到 无底渊")
                && event.actors.iter().any(|actor| actor == "char_tao")
                && event.location.as_deref() == Some("loc_abyss")
        }));

        let checkpoint = world.checkpoint_json().expect("checkpoint serializes");
        let restored = WorldModel::from_checkpoint_json(&checkpoint).expect("valid movement replays");
        assert_eq!(restored.snapshot_json(), world.snapshot_json());
        assert_eq!(world.fork().snapshot_json(), world.snapshot_json());

        let branched = world.branch(|rules| rules.energy_density = 0.91);
        assert_eq!(branched.current_tick(), world.current_tick());
        assert_eq!(
            branched.snapshot().entities.iter().find(|entity| entity.id == "char_tao").unwrap().location_id.as_deref(),
            Some("loc_abyss"),
            "branch replay must preserve movement history"
        );
    }

    #[test]
    fn movement_rejects_missing_route_without_mutating_world() {
        let mut world = WorldModel::new_shitian(20261018);
        let before = world.snapshot_json();
        let error = world.move_character("char_shikong", "loc_abyss").expect_err("no direct route exists");
        assert!(error.contains("no authored route"));
        assert_eq!(world.snapshot_json(), before, "failed movement must not mutate state or events");
        assert!(world.move_character("missing-character", "loc_city").is_err());
        assert_eq!(world.snapshot_json(), before);
    }

    #[test]
    fn tampered_movement_checkpoint_fails_closed() {
        let mut world = WorldModel::new_shitian(20261019);
        world.move_character("char_tao", "loc_abyss").unwrap();
        let mut checkpoint: serde_json::Value =
            serde_json::from_str(&world.checkpoint_json().unwrap()).unwrap();
        checkpoint["operations"][0]["MoveCharacter"]["destination_id"] =
            serde_json::Value::String("loc_missing".to_string());
        assert!(
            WorldModel::from_checkpoint_json(&checkpoint.to_string()).is_err(),
            "invalid route in an untrusted checkpoint must not restore silently"
        );
    }

    #[test]
    fn movement_does_not_change_tick_or_considered_conservation_state() {
        let mut world = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 20261020);
        // Generic worlds have no authored route graph, so validation fails closed before any mutation.
        let before_tick = world.current_tick();
        let before = world.snapshot_json();
        assert!(world.move_character("char_tao", "loc_abyss").is_err());
        assert_eq!(world.current_tick(), before_tick);
        assert_eq!(world.snapshot_json(), before);
    }
}


#[cfg(test)]
mod shortest_route_tests {
    use super::WorldModel;

    #[test]
    fn multi_hop_travel_uses_a_deterministic_authored_path_and_replays() {
        let mut world = WorldModel::new_shitian(20261021);
        let route = world.plan_location_route("loc_hunt", "loc_abyss").unwrap();
        assert_eq!(route, vec!["loc_hunt", "loc_city", "loc_abyss"]);
        assert_eq!(world.plan_location_route("loc_hunt", "loc_abyss").unwrap(), route);
        let tick_before = world.current_tick();
        let event_count_before = world.event_count();

        let actual_path = world.travel_character("char_shikong", "loc_abyss").unwrap();
        assert_eq!(actual_path, route);
        assert_eq!(world.current_tick(), tick_before, "travel must not advance world time implicitly");
        assert_eq!(world.event_count(), event_count_before + 2, "each traversed edge must be auditable");
        assert_eq!(
            world.snapshot().entities.iter().find(|e| e.id == "char_shikong").unwrap().location_id.as_deref(),
            Some("loc_abyss")
        );

        let restored = WorldModel::from_checkpoint_json(&world.checkpoint_json().unwrap()).unwrap();
        assert_eq!(restored.snapshot_json(), world.snapshot_json());
        assert_eq!(world.fork().snapshot_json(), world.snapshot_json());
        assert_eq!(
            world.branch(|rules| rules.energy_density = 0.92)
                .snapshot().entities.iter().find(|e| e.id == "char_shikong").unwrap().location_id.as_deref(),
            Some("loc_abyss")
        );
    }

    #[test]
    fn simulated_travel_advances_time_consumes_supplies_and_replays() {
        let mut preset = GenrePreset::shitian();
        preset.devourer_ids.clear();
        preset.appetite_base = 0.0;
        let mut world = WorldModel::from_preset(preset, 20261023);
        let initial_tick = world.current_tick();
        let initial_supplies = world.snapshot().travel.supplies_remaining;
        let before_preview = world.snapshot_json();
        let estimate = world.plan_travel_character("char_shikong", "loc_abyss").unwrap();
        assert_eq!(world.snapshot_json(), before_preview, "planning must not mutate the live world");
        assert!(estimate.can_afford);
        assert!(estimate.supplies_required > 0.0);
        assert!(estimate.estimated_ticks > 0);
        let report = world.simulate_travel_character("char_shikong", "loc_abyss").unwrap();
        assert_eq!(report.route_cost, estimate.route_cost);
        assert_eq!(report.estimated_ticks, estimate.estimated_ticks);
        assert_eq!(report.supplies_consumed, estimate.supplies_required);

        assert!(report.completed, "the seeded test journey should reach its destination");
        assert_eq!(report.path, vec!["loc_hunt", "loc_city", "loc_abyss"]);
        assert!(report.estimated_ticks > 0);
        assert_eq!(report.elapsed_ticks, report.estimated_ticks);
        assert_eq!(world.current_tick(), initial_tick + report.elapsed_ticks);
        assert!(report.route_cost > 0.0);
        assert!(report.supplies_consumed > 0.0);
        assert!(report.supplies_remaining < initial_supplies);
        assert_eq!(world.snapshot().travel.journeys_attempted, 1);
        assert_eq!(world.snapshot().travel.journeys_completed, 1);
        assert_eq!(world.snapshot().travel.last_encounters, report.encounters.len());
        assert_eq!(
            world.snapshot().entities.iter().find(|e| e.id == "char_shikong").unwrap().location_id.as_deref(),
            Some("loc_abyss")
        );

        let restored = WorldModel::from_checkpoint_json(&world.checkpoint_json().unwrap()).unwrap();
        assert_eq!(restored.snapshot_json(), world.snapshot_json(),
            "travel budgets, tick advancement and encounter events must replay deterministically");
        assert_eq!(world.fork().snapshot_json(), world.snapshot_json());
    }

    #[test]
    fn simulated_travel_rejects_insufficient_supplies_without_mutating_world() {
        let mut world = WorldModel::new_shitian(20261024);
        world.travel.supplies_remaining = 0.0;
        let before = world.snapshot_json();
        let estimate = world.plan_travel_character("char_shikong", "loc_abyss").unwrap();
        assert!(!estimate.can_afford);
        assert!(estimate.supplies_shortfall > 0.0);
        let tick_before = world.current_tick();
        assert!(world.simulate_travel_character("char_shikong", "loc_abyss")
            .unwrap_err().contains("insufficient travel supplies"));
        assert_eq!(world.snapshot_json(), before);
        assert_eq!(world.current_tick(), tick_before);
    }

    #[test]
    fn route_planner_rejects_unknown_and_disconnected_destinations() {
        let mut world = WorldModel::new_shitian(20261022);
        assert!(world.plan_location_route("loc_hunt", "loc_missing").is_err());
        assert!(world.plan_location_route("loc_missing", "loc_city").is_err());
        let before = world.snapshot_json();
        assert!(world.travel_character("char_shikong", "loc_missing").is_err());
        assert_eq!(world.snapshot_json(), before);
    }
}
