//! # 题材预设（GenrePreset）—— 配置驱动的世界构造
//!
//! 把「世界设定」从**硬编码函数**（`new_shitian` 里写死的 7 个实体）降级为**数据**。
//! 引擎对具体题材零硬编码：一份 `GenrePreset` 即可构造任意确定性世界——
//! `genre_id` 仅用于取 `world_rules_for_genre_id` 的中性/特定默认，其余（势力 / 角色 /
//! 地点 / 调参 / 模块）全部来自本结构体。
//!
//! 这正是「换一本小说」的答案：写一份 preset（数据），不动引擎、不手写世界。
//! 食天只是 `GenrePreset::shitian()` 一个实例——题材无关铁律的工程实证。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use wanxiang_types::{Character, EntityManager, Faction, Gender, Location, WorldPosition};

/// 地点预设（确定性 id）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocationSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub resource_richness: f64,
    #[serde(default)]
    pub danger_level: f64,
    /// 显式地点通路目标。只保留预设中存在的 id；不会自动补造缺失关系。
    #[serde(default)]
    pub connected_ids: Vec<String>,
    /// Optional local simulation coordinates; never interpreted as latitude/longitude.
    #[serde(default)]
    pub position: Option<WorldPosition>,
    /// Optional author-defined terrain/biome label.
    #[serde(default)]
    pub terrain: Option<String>,
    /// Base movement cost for entering this location (non-negative model units).
    #[serde(default)]
    pub travel_cost: Option<f64>,
}

/// 势力预设（确定性 id）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactionSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub territory_ids: Vec<String>,
    /// 资源键值对（如 灵石:5000.0）。能量守恒读取 `total_power`，资源用于经济子系统。
    #[serde(default)]
    pub resources: HashMap<String, f64>,
}

/// 角色预设（确定性 id）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterSpec {
    pub id: String,
    pub name: String,
    pub faction_id: String,
    /// 初始驻地（可选；引用不存在的地点时构造器会安全忽略）。
    #[serde(default)]
    pub location_id: Option<String>,
    /// 性别（无默认值，JSON 须显式给出；代码构造时给出）。
    pub gender: Gender,
    #[serde(default)]
    pub power_value: f64,
    #[serde(default)]
    pub power_rank: i32,
}

/// 世界调参（覆盖 `world_rules_for_genre_id` 默认；本结构手动 Default = 0.5，与 `WorldRules::default` 对齐）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldTuning {
    pub energy_density: f64,
    pub resource_abundance: f64,
    pub chaos_level: f64,
    pub order_level: f64,
    pub life_fertility: f64,
}

impl Default for WorldTuning {
    fn default() -> Self {
        Self {
            energy_density: 0.5,
            resource_abundance: 0.5,
            chaos_level: 0.5,
            order_level: 0.5,
            life_fertility: 0.5,
        }
    }
}

/// 题材预设：一份「配置实例」即可构造任意确定性世界。
///
/// 换一本小说 = 写一份 `GenrePreset`（数据），不动引擎、不手写世界。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenrePreset {
    /// 题材 id（仅用于取 `world_rules_for_genre_id` 的默认规则；未知题材给中性默认）。
    pub genre_id: String,
    /// 人类可读标题（如「食天界」）。
    pub title: String,
    #[serde(default)]
    pub world_name: String,
    #[serde(default)]
    pub locations: Vec<LocationSpec>,
    #[serde(default)]
    pub factions: Vec<FactionSpec>,
    #[serde(default)]
    pub characters: Vec<CharacterSpec>,
    /// 模块名列表（如 "devour" / "cultivation"），仅作元数据 / 前端展示；
    /// 实际行为由 `devourer_ids` 驱动（非空才接入吞噬动力学）。
    #[serde(default)]
    pub modules: Vec<String>,
    /// 默认吞噬者（为空则不接入吞噬动力学）。
    #[serde(default)]
    pub devourer_ids: Vec<String>,
    #[serde(default)]
    pub appetite_base: f64,
    #[serde(default)]
    pub enable_cognition: (bool, bool, bool),
    #[serde(default)]
    pub tuning: WorldTuning,
}

impl GenrePreset {
    /// 通用空预设：不构造任何实体、不接入吞噬。用于 `WorldModel::new` 等通用构造。
    pub fn generic() -> Self {
        Self {
            genre_id: "custom".to_string(),
            title: "通用世界".to_string(),
            // B8 (R3 安全可改)：通用/自定义世界真正取名字源改为中性 "世界"，
            // 与 `world_rules_for_genre_id("custom")` 的中性默认对齐；不再写回 "太虚界"
            // （否则 `from_preset` 会用预设名覆盖已中性化的规则名，重新泄漏仙侠名）。
            world_name: "世界".to_string(),
            locations: vec![],
            factions: vec![],
            characters: vec![],
            modules: vec![],
            devourer_ids: vec![],
            appetite_base: 0.0,
            enable_cognition: (false, false, false),
            tuning: WorldTuning::default(),
        }
    }

    /// 食天界预设（第一条小说线锚定实例）。引擎对食天零硬编码，本结构体即「配置实例」实证。
    pub fn shitian() -> Self {
        Self {
            genre_id: "shitian".to_string(),
            title: "食天界".to_string(),
            world_name: "食天界".to_string(),
            locations: vec![
                LocationSpec {
                    id: "loc_hunt".into(),
                    name: "灵墟猎场".into(),
                    environment: "灵物富集、万宗争夺的禁苑".into(),
                    resource_richness: 0.9,
                    danger_level: 0.7,
                    connected_ids: vec!["loc_city".into()],
                    position: Some(WorldPosition::new(-6.0, 0.0, 2.0)),
                    terrain: Some("禁苑".into()),
                    travel_cost: Some(1.3),
                },
                LocationSpec {
                    id: "loc_city".into(),
                    name: "食天城".into(),
                    environment: "中央都会，百味汇流".into(),
                    resource_richness: 0.8,
                    danger_level: 0.3,
                    connected_ids: vec!["loc_hunt".into(), "loc_abyss".into()],
                    position: Some(WorldPosition::new(0.0, 0.0, 0.0)),
                    terrain: Some("城镇".into()),
                    travel_cost: Some(1.0),
                },
                LocationSpec {
                    id: "loc_abyss".into(),
                    name: "无底渊".into(),
                    environment: "吞噬陷阱，坠者成饵的禁地".into(),
                    resource_richness: 0.6,
                    danger_level: 0.9,
                    connected_ids: vec!["loc_city".into()],
                    position: Some(WorldPosition::new(5.0, 2.0, -3.0)),
                    terrain: Some("深渊".into()),
                    travel_cost: Some(2.2),
                },
            ],
            factions: vec![
                FactionSpec {
                    id: "fac_tunling".into(),
                    name: "吞灵宗".into(),
                    territory_ids: vec!["loc_hunt".into()],
                    resources: res("灵石", 5000.0),
                },
                FactionSpec {
                    id: "fac_shimeng".into(),
                    name: "散修食盟".into(),
                    territory_ids: vec!["loc_city".into()],
                    resources: res("灵石", 1800.0),
                },
                FactionSpec {
                    id: "fac_mingge".into(),
                    name: "命阁".into(),
                    territory_ids: vec!["loc_abyss".into()],
                    resources: res("灵石", 1200.0),
                },
            ],
            characters: vec![
                CharacterSpec {
                    id: "char_tao".into(),
                    name: "饕".into(),
                    faction_id: "fac_shimeng".into(),
                    location_id: Some("loc_city".into()),
                    gender: Gender::Male,
                    power_value: 650.0,
                    power_rank: 5,
                },
                CharacterSpec {
                    id: "char_shikong".into(),
                    name: "噬空".into(),
                    faction_id: "fac_tunling".into(),
                    location_id: Some("loc_hunt".into()),
                    gender: Gender::Male,
                    power_value: 880.0,
                    power_rank: 5,
                },
                CharacterSpec {
                    id: "char_baiwei".into(),
                    name: "百味".into(),
                    faction_id: "fac_shimeng".into(),
                    location_id: Some("loc_city".into()),
                    gender: Gender::Female,
                    power_value: 520.0,
                    power_rank: 3,
                },
                CharacterSpec {
                    id: "char_siming".into(),
                    name: "司命".into(),
                    faction_id: "fac_mingge".into(),
                    location_id: Some("loc_abyss".into()),
                    gender: Gender::Female,
                    power_value: 430.0,
                    power_rank: 3,
                },
            ],
            modules: vec!["devour".into(), "cultivation".into()],
            devourer_ids: vec!["char_tao".into()],
            appetite_base: 12.0,
            enable_cognition: (true, true, true),
            tuning: WorldTuning {
                energy_density: 0.85,
                resource_abundance: 0.9,
                chaos_level: 0.18,
                order_level: 0.85,
                life_fertility: 0.6,
            },
        }
    }

    /// 已注册的内置预设清单。新增题材 = 在此追加一个 `GenrePreset::{name}()`。
    pub fn registry() -> Vec<GenrePreset> {
        vec![Self::shitian()]
    }

    pub fn faction_ids(&self) -> Vec<String> {
        self.factions.iter().map(|f| f.id.clone()).collect()
    }

    pub fn character_ids(&self) -> Vec<String> {
        self.characters.iter().map(|c| c.id.clone()).collect()
    }

    /// 由预设构造确定性实体（跳过默认人口 spawn；命名角色高 death_protection 全程存活）。
    pub fn build_entities(&self) -> EntityManager {
        let mut em = EntityManager::new();
        let valid_location_ids: HashSet<&str> =
            self.locations.iter().map(|location| location.id.as_str()).collect();
        for l in &self.locations {
            let mut loc = Location::new(l.name.clone(), l.environment.clone());
            loc.id = l.id.clone();
            loc.resource_richness = l.resource_richness;
            loc.danger_level = l.danger_level;
            loc.position = l.position.filter(|position| position.is_finite());
            loc.terrain = l.terrain.clone();
            loc.travel_cost = l.travel_cost.filter(|cost| cost.is_finite() && *cost >= 0.0);
            em.add_location(loc);
        }
        // Import authored connections only after all endpoints exist. Invalid/self links are ignored.
        for spec in &self.locations {
            if let Some(location) = em.locations.get_mut(&spec.id) {
                let mut targets = spec.connected_ids.clone();
                targets.sort_unstable();
                targets.dedup();
                for target in targets {
                    if target != spec.id && valid_location_ids.contains(target.as_str()) {
                        location.connect(target);
                    }
                }
            }
        }
        for f in &self.factions {
            let mut fac = Faction::new(f.name.clone());
            fac.id = f.id.clone();
            fac.territory_ids = f.territory_ids.clone();
            for (k, v) in &f.resources {
                fac.resources.insert(k.clone(), *v);
            }
            em.add_faction(fac);
        }
        for c in &self.characters {
            let mut ch = Character::new(c.name.clone(), c.gender);
            ch.id = c.id.clone();
            ch.power_value = c.power_value;
            ch.power_rank = c.power_rank;
            ch.faction_id = Some(c.faction_id.clone());
            if let Some(location_id) = c.location_id.as_ref() {
                if valid_location_ids.contains(location_id.as_str()) {
                    ch.location_id = Some(location_id.clone());
                }
            }
            ch.death_protection_days = 1_000_000;
            if let Some(fac) = em.factions.get_mut(&c.faction_id) {
                fac.members.push(c.id.clone());
            }
            em.add_character(ch);
        }
        em
    }

    /// 强制维持预设框架（每 tick 调用）：移除泛化实体 → 复原基准势力 → 角色归位。
    /// 通用空预设（无势力）时为 no-op，不影响通用世界。
    pub fn enforce_frame(&self, em: &mut EntityManager) {
        if self.factions.is_empty() {
            return;
        }
        let fac_ids = self.faction_ids();
        let char_ids = self.character_ids();
        let remove_fac: Vec<String> = em
            .factions
            .ids_str()
            .into_iter()
            .filter(|id| !fac_ids.contains(id))
            .collect();
        for id in remove_fac {
            em.factions.remove(&id);
        }
        let remove_ch: Vec<String> = em
            .characters
            .ids_str()
            .into_iter()
            .filter(|id| !char_ids.contains(id))
            .collect();
        for id in remove_ch {
            em.characters.remove(&id);
        }
        for f in &self.factions {
            let mut fac = Faction::new(f.name.clone());
            fac.id = f.id.clone();
            fac.territory_ids = f.territory_ids.clone();
            for (k, v) in &f.resources {
                fac.resources.insert(k.clone(), *v);
            }
            em.add_faction(fac);
        }
        for c in &self.characters {
            if let Some(ch) = em.characters.get_mut(&c.id) {
                ch.faction_id = Some(c.faction_id.clone());
            }
            if let Some(fac) = em.factions.get_mut(&c.faction_id) {
                let cid = c.id.clone();
                if !fac.members.contains(&cid) {
                    fac.members.push(cid);
                }
            }
        }
    }
}

/// 构造单键资源表（预设里最常见的「一种资源 + 一个基准量」）。
fn res(key: &str, amount: f64) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert(key.to_string(), amount);
    m
}

/// B8 (R3 安全可改) 回归测试：通用/自定义世界的取名字源必须中性，不再泄漏「太虚界」。
///
/// 背景：`WorldModel::new` 与所有通用构造都走 `GenrePreset::generic()`，而 `from_preset`
/// 在 `preset.world_name` 非空时会用预设名覆盖 `world_rules_for_genre_id("custom")` 已中性化的
/// "世界"。若 generic 预设仍写 "太虚界"，B4 的中性默认会被这条路径重新泄漏。
/// 注：本测试依赖 `wanxiang_sdk` 工作区，本沙箱不编译，待真机 `cargo test --workspace` 复核。
#[cfg(test)]
mod b8_neutral_world_name_tests {
    use super::*;
    use crate::WorldModel;

    #[test]
    fn generic_preset_world_name_is_neutral() {
        assert_eq!(GenrePreset::generic().world_name, "世界");
    }

    #[test]
    fn from_preset_generic_yields_neutral_world_name() {
        // 覆盖逻辑写回的应是中性 "世界"，而非预设里的旧 "太虚界"。
        let model = WorldModel::from_preset(GenrePreset::generic(), 42);
        assert_eq!(model.snapshot().rules.world_name, "世界");
    }
}


#[cfg(test)]
mod location_preset_tests {
    use super::*;

    #[test]
    fn shitian_preset_exports_authored_bidirectional_locations_and_residents() {
        let preset = GenrePreset::shitian();
        let ids: HashSet<&str> = preset.locations.iter().map(|location| location.id.as_str()).collect();
        assert_eq!(preset.locations.len(), 3);
        for location in &preset.locations {
            for target in &location.connected_ids {
                assert!(ids.contains(target.as_str()), "unknown location edge target {target}");
                let reverse = preset.locations.iter()
                    .find(|candidate| candidate.id == *target)
                    .expect("target is declared");
                assert!(reverse.connected_ids.contains(&location.id),
                    "preset route {} ↔ {} must be reciprocal", location.id, target);
            }
        }

        let world = preset.build_entities();
        for character in &preset.characters {
            let entity = world.characters.get(&character.id).expect("preset character is built");
            assert!(entity.location_id.is_some(), "preset character {} should have an authored location", character.id);
            assert!(entity.location_id.as_ref().is_some_and(|id| ids.contains(id.as_str())));
        }
        assert!(world.locations.values().iter().any(|location| !location.connected_ids.is_empty()));
    }
}
