//! serde 输出 DTO —— 对齐 EI 官方 JSON 序列化形状（GW2EIJSON + JsonModels）。
//!
//! 规则（research/ei-json-dto.md + ei-golden-json-shape.md 实测，不可变通）：
//! - 键名逐字段 `#[serde(rename = "...")]`（System.Text.Json lowercase-prefix 策略
//!   不均匀，`eiLogID`/`gW2Build`/`wvWMapData` 等不能靠 rename_all 生成）；
//! - null/空集合整体省略 = `Option` + `skip_serializing_if`；值类型 0/false/空串
//!   必输出（C# 无 `DefaultIgnoreCondition=WhenWritingNull` 之外的条件）；
//! - f64 字段统一 `#[serde(serialize_with = "crate::ser::ser_f64")]`：
//!   C# System.Text.Json 输出最短往返 + 整数值不带 `.0`（golden 实测 `0` 而非
//!   `0.0`），NaN/±Infinity 输出为字符串 `"NaN"`/`"Infinity"`/`"-Infinity"`；
//! - 数字类型宽度以「数值精确」为准（ushort/uint/ulong 在 JSON 里就是数字，
//!   负 id 存在 → i64）。

use serde::Serialize;

use crate::dto_b::{JsonNpc, JsonPlayer};
use crate::ser::ser_f64;

/// `JsonLog`（JsonLog.cs）。字段顺序按官方输出实测键序（对拍 Value 比较无关，
/// 但保持稳定以便未来字节级比对）。
#[derive(Serialize)]
pub struct JsonLog {
    #[serde(rename = "parsingSettings")]
    pub parsing_settings: ParsingSettings,
    #[serde(rename = "eliteInsightsVersion")]
    pub elite_insights_version: String,
    #[serde(rename = "triggerID")]
    pub trigger_id: i64,
    #[serde(rename = "isInstanceLog")]
    pub is_instance_log: bool,
    #[serde(rename = "eiEncounterID")]
    pub ei_encounter_id: i64,
    #[serde(rename = "eiLogID")]
    pub ei_log_id: i64,
    #[serde(rename = "mapID")]
    pub map_id: i64,
    #[serde(rename = "fightName")]
    pub fight_name: String,
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "fightIcon")]
    pub fight_icon: String,
    #[serde(rename = "icon")]
    pub icon: String,
    #[serde(rename = "arcVersion")]
    pub arc_version: String,
    #[serde(rename = "arcRevision")]
    pub arc_revision: i64,
    #[serde(rename = "gW2Build")]
    pub gw2_build: i64,
    #[serde(rename = "language")]
    pub language: String,
    #[serde(rename = "fractalScale")]
    pub fractal_scale: i64,
    #[serde(rename = "region")]
    pub region: String,
    #[serde(rename = "languageID")]
    pub language_id: i64,
    #[serde(rename = "recordedBy")]
    pub recorded_by: String,
    #[serde(rename = "recordedAccountBy")]
    pub recorded_account_by: String,
    #[serde(rename = "timeStart")]
    pub time_start: String,
    #[serde(rename = "timeEnd")]
    pub time_end: String,
    #[serde(rename = "timeStartStd")]
    pub time_start_std: String,
    #[serde(rename = "timeEndStd")]
    pub time_end_std: String,
    #[serde(rename = "duration")]
    pub duration: String,
    #[serde(rename = "durationMS")]
    pub duration_ms: i64,
    #[serde(rename = "logStartOffset")]
    pub log_start_offset: i64,
    #[serde(rename = "instanceTimeStartStd", skip_serializing_if = "Option::is_none")]
    pub instance_time_start_std: Option<String>,
    #[serde(rename = "instanceIP", skip_serializing_if = "Option::is_none")]
    pub instance_ip: Option<String>,
    #[serde(rename = "instancePrivacy")]
    pub instance_privacy: String,
    #[serde(rename = "targetless")]
    pub targetless: bool,
    #[serde(rename = "success")]
    pub success: bool,
    #[serde(rename = "isCM")]
    pub is_cm: bool,
    #[serde(rename = "isLegendaryCM")]
    pub is_legendary_cm: bool,
    #[serde(rename = "isLateStart")]
    pub is_late_start: bool,
    #[serde(rename = "missingPreEvent")]
    pub missing_pre_event: bool,
    #[serde(rename = "anonymous")]
    pub anonymous: bool,
    #[serde(rename = "detailedWvW")]
    pub detailed_wvw: bool,
    #[serde(rename = "targets")]
    pub targets: Vec<JsonNpc>,
    #[serde(rename = "players")]
    pub players: Vec<JsonPlayer>,
    #[serde(rename = "phases")]
    pub phases: Vec<JsonPhase>,
    #[serde(rename = "mechanics", skip_serializing_if = "Option::is_none")]
    pub mechanics: Option<Vec<JsonMechanics>>,
    #[serde(rename = "uploadLinks")]
    pub upload_links: Vec<String>,
    #[serde(rename = "skillMap")]
    pub skill_map: std::collections::BTreeMap<String, SkillDesc>,
    #[serde(rename = "buffMap")]
    pub buff_map: std::collections::BTreeMap<String, BuffDesc>,
    #[serde(rename = "damageModMap")]
    pub damage_mod_map: std::collections::BTreeMap<String, DamageModDesc>,
    #[serde(rename = "teamMap")]
    pub team_map: std::collections::BTreeMap<String, TeamDesc>,
    #[serde(rename = "personalBuffs")]
    pub personal_buffs: std::collections::BTreeMap<String, Vec<i64>>,
    #[serde(rename = "personalDamageMods")]
    pub personal_damage_mods: std::collections::BTreeMap<String, Vec<i64>>,
    #[serde(rename = "logErrors", skip_serializing_if = "Option::is_none")]
    pub log_errors: Option<Vec<String>>,
    #[serde(rename = "combatReplayMetaData", skip_serializing_if = "Option::is_none")]
    pub combat_replay_meta_data: Option<CombatReplayMetaData>,
    #[serde(rename = "wvWMapData", skip_serializing_if = "Option::is_none")]
    pub wvw_map_data: Option<WvwMapData>,
}

#[derive(Serialize)]
pub struct ParsingSettings {
    #[serde(rename = "parseExtensions")]
    pub parse_extensions: bool,
    #[serde(rename = "computePhases")]
    pub compute_phases: bool,
    #[serde(rename = "computeCombatReplay")]
    pub compute_combat_replay: bool,
    #[serde(rename = "computeDamageModifiers")]
    pub compute_damage_modifiers: bool,
    #[serde(rename = "computeDamage")]
    pub compute_damage: bool,
    #[serde(rename = "computeCast")]
    pub compute_cast: bool,
    #[serde(rename = "computeBuff")]
    pub compute_buff: bool,
    #[serde(rename = "computeMechanics")]
    pub compute_mechanics: bool,
}

// ===== phases =====
#[derive(Serialize)]
pub struct JsonPhase {
    #[serde(rename = "start")]
    pub start: i64,
    #[serde(rename = "end")]
    pub end: i64,
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "targets")]
    pub targets: Vec<i64>,
    #[serde(rename = "secondaryTargets")]
    pub secondary_targets: Vec<i64>,
    #[serde(rename = "targetPriorities")]
    pub target_priorities: std::collections::BTreeMap<String, String>,
    #[serde(rename = "phaseType")]
    pub phase_type: String,
    #[serde(rename = "breakbarPhase")]
    pub breakbar_phase: bool,
    #[serde(rename = "subPhases", skip_serializing_if = "Option::is_none")]
    pub sub_phases: Option<Vec<i64>>,
    #[serde(rename = "success", skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(rename = "isLegendaryCM", skip_serializing_if = "Option::is_none")]
    pub is_legendary_cm: Option<bool>,
    #[serde(rename = "isCM", skip_serializing_if = "Option::is_none")]
    pub is_cm: Option<bool>,
    #[serde(rename = "eiEncounterID", skip_serializing_if = "Option::is_none")]
    pub ei_encounter_id: Option<i64>,
    #[serde(rename = "encounterIcon", skip_serializing_if = "Option::is_none")]
    pub encounter_icon: Option<String>,
    #[serde(rename = "encounterIsLateStart", skip_serializing_if = "Option::is_none")]
    pub encounter_is_late_start: Option<bool>,
    #[serde(rename = "encounterMissingPreEvent", skip_serializing_if = "Option::is_none")]
    pub encounter_missing_pre_event: Option<bool>,
    #[serde(rename = "encounterPhase", skip_serializing_if = "Option::is_none")]
    pub encounter_phase: Option<i64>,
    #[serde(rename = "breakbarRecovered", skip_serializing_if = "Option::is_none")]
    pub breakbar_recovered: Option<bool>,
    #[serde(rename = "breakbarActive", skip_serializing_if = "Option::is_none")]
    pub breakbar_active: Option<i64>,
}

// ===== mechanics =====
#[derive(Serialize)]
pub struct JsonMechanics {
    #[serde(rename = "mechanicsData")]
    pub mechanics_data: Vec<JsonMechanic>,
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(rename = "description")]
    pub description: String,
    #[serde(rename = "internalCooldown", skip_serializing_if = "Option::is_none")]
    pub internal_cooldown: Option<i64>,
    #[serde(rename = "isAchievementEligibility")]
    pub is_achievement_eligibility: bool,
    #[serde(rename = "severity")]
    pub severity: String,
}

#[derive(Serialize)]
pub struct JsonMechanic {
    #[serde(rename = "time")]
    pub time: i64,
    #[serde(rename = "actor")]
    pub actor: String,
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "instid")]
    pub instid: i64,
    #[serde(rename = "weight")]
    pub weight: f64,
}

// ===== 四张 map（收集器）=====
#[derive(Serialize)]
pub struct SkillDesc {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "autoAttack")]
    pub auto_attack: bool,
    #[serde(rename = "canCrit")]
    pub can_crit: bool,
    #[serde(rename = "icon")]
    pub icon: String,
    #[serde(rename = "isSwap")]
    pub is_swap: bool,
    #[serde(rename = "isInstantCast")]
    pub is_instant_cast: bool,
    #[serde(rename = "isTraitProc")]
    pub is_trait_proc: bool,
    #[serde(rename = "isUnconditionalProc")]
    pub is_unconditional_proc: bool,
    #[serde(rename = "isGearProc")]
    pub is_gear_proc: bool,
    #[serde(rename = "isNotAccurate")]
    pub is_not_accurate: bool,
    #[serde(rename = "conversionBasedHealing")]
    pub conversion_based_healing: bool,
    #[serde(rename = "hybridHealing")]
    pub hybrid_healing: bool,
}

#[derive(Serialize)]
pub struct BuffDesc {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "classification", skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(rename = "icon")]
    pub icon: String,
    #[serde(rename = "stacking")]
    pub stacking: bool,
    #[serde(rename = "conversionBasedHealing")]
    pub conversion_based_healing: bool,
    #[serde(rename = "hybridHealing")]
    pub hybrid_healing: bool,
    #[serde(rename = "descriptions", skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct DamageModDesc {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "icon")]
    pub icon: String,
    #[serde(rename = "description")]
    pub description: String,
    #[serde(rename = "nonMultiplier")]
    pub non_multiplier: bool,
    #[serde(rename = "isCounter")]
    pub is_counter: bool,
    #[serde(rename = "skillBased")]
    pub skill_based: bool,
    #[serde(rename = "approximate")]
    pub approximate: bool,
    #[serde(rename = "incoming")]
    pub incoming: bool,
}

#[derive(Serialize)]
pub struct TeamDesc {
    #[serde(rename = "guid")]
    pub guid: String,
}

#[derive(Serialize)]
pub struct CombatReplayMetaData {
    #[serde(rename = "inchToPixel", serialize_with = "ser_f64")]
    pub inch_to_pixel: f64,
    #[serde(rename = "pollingRate")]
    pub polling_rate: i64,
    #[serde(rename = "sizes")]
    pub sizes: Vec<i64>,
    #[serde(rename = "maps")]
    pub maps: Vec<CombatReplayMapMeta>,
}

#[derive(Serialize)]
pub struct CombatReplayMapMeta {
    #[serde(rename = "url")]
    pub url: String,
    #[serde(rename = "interval")]
    pub interval: Vec<i64>,
    #[serde(rename = "position")]
    pub position: Vec<i64>,
}

#[derive(Serialize)]
pub struct WvwMapData {
    #[serde(rename = "redShardID")]
    pub red_shard_id: i64,
    #[serde(rename = "blueShardID")]
    pub blue_shard_id: i64,
    #[serde(rename = "greenShardID")]
    pub green_shard_id: i64,
    #[serde(rename = "redTeamID")]
    pub red_team_id: i64,
    #[serde(rename = "blueTeamID")]
    pub blue_team_id: i64,
    #[serde(rename = "greenTeamID")]
    pub green_team_id: i64,
    #[serde(rename = "objectiveData")]
    pub objective_data: Vec<WvwObjectiveData>,
}

#[derive(Serialize)]
pub struct WvwObjectiveData {
    #[serde(rename = "mapID")]
    pub map_id: i64,
    #[serde(rename = "objectiveID")]
    pub objective_id: i64,
    #[serde(rename = "objectiveType")]
    pub objective_type: String,
    #[serde(rename = "owners")]
    pub owners: Vec<Vec<i64>>,
}
