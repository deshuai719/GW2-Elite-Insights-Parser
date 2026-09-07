//! actor DTO（JsonActors + JsonActorUtilities 的 P2 块）。
//!
//! 数字类型选择（golden 实测,见 dto_a.rs 头部规则）：
//! - C# double 字段 → `f64` + `ser_f64`（整数值输出整数 token,与 C# 一致）;
//! - C# int/long 字段 → `i64`;时间轴/1s 桶/计数恒 int。
//!
//! OOS(P3+)块不在 DTO 中出现(序列化省略;对拍 OOS 剔除):buff 统计 16
//! 组(buffUptimes/buffVolumes/self/group/offGroup/squad ×Active)、damage
//! Modifiers×4、EXT、combatReplayData、boonsStates、conditionsStates、
//! activeCombatMinions、minions、activeRangerPets、commanderTagStates、
//! activeClones 等;damageModifiers 在 C# 为恒空列表照常输出,本侧省略。

use serde::Serialize;

use crate::ser::ser_f64;

// ===== JsonPlayer（JsonPlayer.cs;键序按官方输出实测）=====
#[derive(Serialize)]
pub struct JsonPlayer {
    #[serde(rename = "account")]
    pub account: String,
    #[serde(rename = "group")]
    pub group: i64,
    #[serde(rename = "hasCommanderTag")]
    pub has_commander_tag: bool,
    #[serde(rename = "profession")]
    pub profession: String,
    #[serde(rename = "friendlyNPC")]
    pub friendly_npc: bool,
    #[serde(rename = "notInSquad")]
    pub not_in_squad: bool,
    #[serde(rename = "guildID", skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(rename = "isEnglobed")]
    pub is_englobed: bool,
    #[serde(rename = "weapons")]
    pub weapons: Vec<String>,
    #[serde(rename = "weaponSets")]
    pub weapon_sets: Vec<JsonWeaponSet>,
    #[serde(rename = "dpsTargets")]
    pub dps_targets: Vec<Vec<JsonDps>>,
    #[serde(rename = "damageTaken1S")]
    pub damage_taken_1s: Vec<Vec<i64>>,
    #[serde(rename = "powerDamageTaken1S")]
    pub power_damage_taken_1s: Vec<Vec<i64>>,
    #[serde(rename = "conditionDamageTaken1S")]
    pub condition_damage_taken_1s: Vec<Vec<i64>>,
    #[serde(rename = "breakbarDamageTaken1S")]
    pub breakbar_damage_taken_1s: Vec<Option<Vec<f64>>>,
    #[serde(rename = "targetDamage1S")]
    pub target_damage_1s: Vec<Vec<Vec<i64>>>,
    #[serde(rename = "targetPowerDamage1S")]
    pub target_power_damage_1s: Vec<Vec<Vec<i64>>>,
    #[serde(rename = "targetConditionDamage1S")]
    pub target_condition_damage_1s: Vec<Vec<Vec<i64>>>,
    #[serde(rename = "targetDamageDist")]
    pub target_damage_dist: Vec<Vec<Vec<JsonDamageDist>>>,
    #[serde(rename = "statsTargets")]
    pub stats_targets: Vec<Vec<JsonGameplayStats>>,
    #[serde(rename = "support")]
    pub support: Vec<JsonPlayerSupport>,
    #[serde(rename = "activeTimes")]
    pub active_times: Vec<i64>,
    // ---- JsonActor 公共块 ----
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "firstAware")]
    pub first_aware: i64,
    #[serde(rename = "lastAware")]
    pub last_aware: i64,
    #[serde(rename = "totalHealth")]
    pub total_health: i64,
    #[serde(rename = "condition")]
    pub condition: i64,
    #[serde(rename = "concentration")]
    pub concentration: i64,
    #[serde(rename = "healing")]
    pub healing: i64,
    #[serde(rename = "toughness")]
    pub toughness: i64,
    #[serde(rename = "hitboxHeight")]
    pub hitbox_height: i64,
    #[serde(rename = "hitboxWidth")]
    pub hitbox_width: i64,
    #[serde(rename = "instanceID")]
    pub instance_id: i64,
    #[serde(rename = "teamID")]
    pub team_id: i64,
    #[serde(rename = "isFake")]
    pub is_fake: bool,
    #[serde(rename = "dpsAll")]
    pub dps_all: Vec<JsonDps>,
    #[serde(rename = "statsAll")]
    pub stats_all: Vec<JsonGameplayStatsAll>,
    #[serde(rename = "defenses")]
    pub defenses: Vec<JsonDefensesAll>,
    #[serde(rename = "totalDamageDist")]
    pub total_damage_dist: Vec<Vec<JsonDamageDist>>,
    #[serde(rename = "totalDamageTaken")]
    pub total_damage_taken: Vec<Vec<JsonDamageDist>>,
    #[serde(rename = "rotation", skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Vec<JsonRotation>>,
    #[serde(rename = "damage1S")]
    pub damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "powerDamage1S")]
    pub power_damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "conditionDamage1S")]
    pub condition_damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "healthPercents")]
    pub health_percents: Vec<Vec<f64>>,
    #[serde(rename = "barrierPercents")]
    pub barrier_percents: Vec<Vec<f64>>,
    #[serde(rename = "consumables", skip_serializing_if = "Option::is_none")]
    pub consumables: Option<Vec<JsonConsumable>>,
    #[serde(rename = "deathRecap", skip_serializing_if = "Option::is_none")]
    pub death_recap: Option<Vec<JsonDeathRecap>>,
}

// ===== JsonNPC（JsonNPC.cs）=====
#[derive(Serialize)]
pub struct JsonNpc {
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "finalHealth")]
    pub final_health: i64,
    #[serde(rename = "finalBarrier")]
    pub final_barrier: i64,
    #[serde(rename = "barrierPercent", serialize_with = "ser_f64")]
    pub barrier_percent: f64,
    #[serde(rename = "healthPercentBurned", serialize_with = "ser_f64")]
    pub health_percent_burned: f64,
    #[serde(rename = "enemyPlayer")]
    pub enemy_player: bool,
    #[serde(rename = "breakbarPercents")]
    pub breakbar_percents: Vec<Vec<f64>>,
    // ---- JsonActor 公共块 ----
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "firstAware")]
    pub first_aware: i64,
    #[serde(rename = "lastAware")]
    pub last_aware: i64,
    #[serde(rename = "totalHealth")]
    pub total_health: i64,
    #[serde(rename = "condition")]
    pub condition: i64,
    #[serde(rename = "concentration")]
    pub concentration: i64,
    #[serde(rename = "healing")]
    pub healing: i64,
    #[serde(rename = "toughness")]
    pub toughness: i64,
    #[serde(rename = "hitboxHeight")]
    pub hitbox_height: i64,
    #[serde(rename = "hitboxWidth")]
    pub hitbox_width: i64,
    #[serde(rename = "instanceID")]
    pub instance_id: i64,
    #[serde(rename = "teamID")]
    pub team_id: i64,
    #[serde(rename = "isFake")]
    pub is_fake: bool,
    #[serde(rename = "dpsAll")]
    pub dps_all: Vec<JsonDps>,
    #[serde(rename = "statsAll")]
    pub stats_all: Vec<JsonGameplayStatsAll>,
    #[serde(rename = "defenses")]
    pub defenses: Vec<JsonDefensesAll>,
    #[serde(rename = "totalDamageDist")]
    pub total_damage_dist: Vec<Vec<JsonDamageDist>>,
    #[serde(rename = "totalDamageTaken")]
    pub total_damage_taken: Vec<Vec<JsonDamageDist>>,
    #[serde(rename = "rotation", skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Vec<JsonRotation>>,
    #[serde(rename = "damage1S")]
    pub damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "powerDamage1S")]
    pub power_damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "conditionDamage1S")]
    pub condition_damage_1s: Vec<Vec<i64>>,
    #[serde(rename = "healthPercents")]
    pub health_percents: Vec<Vec<f64>>,
    #[serde(rename = "barrierPercents")]
    pub barrier_percents: Vec<Vec<f64>>,
}

// ===== JsonActorUtilities =====

/// JsonDPS（JsonStatistics.JsonDPS：14 键;breakbar 两个是 double）。
#[derive(Serialize)]
pub struct JsonDps {
    #[serde(rename = "dps")]
    pub dps: i64,
    #[serde(rename = "damage")]
    pub damage: i64,
    #[serde(rename = "condiDps")]
    pub condi_dps: i64,
    #[serde(rename = "condiDamage")]
    pub condi_damage: i64,
    #[serde(rename = "powerDps")]
    pub power_dps: i64,
    #[serde(rename = "powerDamage")]
    pub power_damage: i64,
    #[serde(rename = "breakbarDamage", serialize_with = "ser_f64")]
    pub breakbar_damage: f64,
    #[serde(rename = "actorDps")]
    pub actor_dps: i64,
    #[serde(rename = "actorDamage")]
    pub actor_damage: i64,
    #[serde(rename = "actorCondiDps")]
    pub actor_condi_dps: i64,
    #[serde(rename = "actorCondiDamage")]
    pub actor_condi_damage: i64,
    #[serde(rename = "actorPowerDps")]
    pub actor_power_dps: i64,
    #[serde(rename = "actorPowerDamage")]
    pub actor_power_damage: i64,
    #[serde(rename = "actorBreakbarDamage", serialize_with = "ser_f64")]
    pub actor_breakbar_damage: f64,
}

/// JsonGameplayStats（statsTargets 的 38 键 = OffensiveStatistics 面）。
#[derive(Serialize)]
pub struct JsonGameplayStats {
    #[serde(rename = "totalDamageCount")]
    pub total_damage_count: i64,
    #[serde(rename = "totalDmg")]
    pub total_dmg: i64,
    #[serde(rename = "directDamageCount")]
    pub direct_damage_count: i64,
    #[serde(rename = "directDmg")]
    pub direct_dmg: i64,
    #[serde(rename = "connectedDamageCount")]
    pub connected_damage_count: i64,
    #[serde(rename = "connectedDmg")]
    pub connected_dmg: i64,
    #[serde(rename = "connectedDirectDamageCount")]
    pub connected_direct_damage_count: i64,
    #[serde(rename = "connectedDirectDmg")]
    pub connected_direct_dmg: i64,
    #[serde(rename = "connectedPowerCount")]
    pub connected_power_count: i64,
    #[serde(rename = "connectedPowerDamage")]
    pub connected_power_damage: i64,
    #[serde(rename = "connectedPowerAbove90HPCount")]
    pub connected_power_above_90_hp_count: i64,
    #[serde(rename = "connectedPowerAbove90HPDamage")]
    pub connected_power_above_90_hp_damage: i64,
    #[serde(rename = "connectedLifeLeechCount")]
    pub connected_life_leech_count: i64,
    #[serde(rename = "connectedLifeLeechDamage")]
    pub connected_life_leech_damage: i64,
    #[serde(rename = "connectedConditionCount")]
    pub connected_condition_count: i64,
    #[serde(rename = "connectedConditionDamage")]
    pub connected_condition_damage: i64,
    #[serde(rename = "connectedConditionAbove90HPCount")]
    pub connected_condition_above_90_hp_count: i64,
    #[serde(rename = "connectedConditionAbove90HPDamage")]
    pub connected_condition_above_90_hp_damage: i64,
    #[serde(rename = "critableDirectDamageCount")]
    pub critable_direct_damage_count: i64,
    #[serde(rename = "criticalRate")]
    pub critical_rate: i64,
    #[serde(rename = "criticalDmg")]
    pub critical_dmg: i64,
    #[serde(rename = "flankingRate")]
    pub flanking_rate: i64,
    #[serde(rename = "againstMovingRate")]
    pub against_moving_rate: i64,
    #[serde(rename = "glanceRate")]
    pub glance_rate: i64,
    #[serde(rename = "missed")]
    pub missed: i64,
    #[serde(rename = "evaded")]
    pub evaded: i64,
    #[serde(rename = "blocked")]
    pub blocked: i64,
    #[serde(rename = "interrupts")]
    pub interrupts: i64,
    #[serde(rename = "invulned")]
    pub invulned: i64,
    #[serde(rename = "killed")]
    pub killed: i64,
    #[serde(rename = "downed")]
    pub downed: i64,
    #[serde(rename = "againstDownedCount")]
    pub against_downed_count: i64,
    #[serde(rename = "againstDownedDamage")]
    pub against_downed_damage: i64,
    #[serde(rename = "downContribution")]
    pub down_contribution: i64,
    #[serde(rename = "appliedCrowdControlDownContribution")]
    pub applied_crowd_control_down_contribution: i64,
    #[serde(rename = "appliedCrowdControlDurationDownContribution", serialize_with = "ser_f64")]
    pub applied_crowd_control_duration_down_contribution: f64,
    #[serde(rename = "appliedCrowdControl")]
    pub applied_crowd_control: i64,
    #[serde(rename = "appliedCrowdControlDuration", serialize_with = "ser_f64")]
    pub applied_crowd_control_duration: f64,
}

/// JsonGameplayStatsAll（statsAll = GameplayStats 13 + Offensive 38）。
#[derive(Serialize)]
pub struct JsonGameplayStatsAll {
    #[serde(rename = "wasted")]
    pub wasted: i64,
    #[serde(rename = "timeWasted", serialize_with = "ser_f64")]
    pub time_wasted: f64,
    #[serde(rename = "saved")]
    pub saved: i64,
    #[serde(rename = "timeSaved", serialize_with = "ser_f64")]
    pub time_saved: f64,
    #[serde(rename = "stackDist", serialize_with = "ser_f64")]
    pub stack_dist: f64,
    #[serde(rename = "distToCom", serialize_with = "ser_f64")]
    pub dist_to_com: f64,
    #[serde(rename = "avgBoons", serialize_with = "ser_f64")]
    pub avg_boons: f64,
    #[serde(rename = "avgActiveBoons", serialize_with = "ser_f64")]
    pub avg_active_boons: f64,
    #[serde(rename = "avgConditions", serialize_with = "ser_f64")]
    pub avg_conditions: f64,
    #[serde(rename = "avgActiveConditions", serialize_with = "ser_f64")]
    pub avg_active_conditions: f64,
    #[serde(rename = "swapCount")]
    pub swap_count: i64,
    #[serde(rename = "skillCastUptime", serialize_with = "ser_f64")]
    pub skill_cast_uptime: f64,
    #[serde(rename = "skillCastUptimeNoAA", serialize_with = "ser_f64")]
    pub skill_cast_uptime_no_aa: f64,
    #[serde(flatten)]
    pub off: JsonGameplayStats,
}

/// JsonDefensesAll（DefenseAllStatistics：40 键;Duration/时间字段为 double）。
#[derive(Serialize)]
pub struct JsonDefensesAll {
    #[serde(rename = "damageTaken")]
    pub damage_taken: i64,
    #[serde(rename = "damageTakenCount")]
    pub damage_taken_count: i64,
    #[serde(rename = "conditionDamageTaken")]
    pub condition_damage_taken: i64,
    #[serde(rename = "conditionDamageTakenCount")]
    pub condition_damage_taken_count: i64,
    #[serde(rename = "powerDamageTaken")]
    pub power_damage_taken: i64,
    #[serde(rename = "powerDamageTakenCount")]
    pub power_damage_taken_count: i64,
    #[serde(rename = "strikeDamageTaken")]
    pub strike_damage_taken: i64,
    #[serde(rename = "strikeDamageTakenCount")]
    pub strike_damage_taken_count: i64,
    #[serde(rename = "lifeLeechDamageTaken")]
    pub life_leech_damage_taken: i64,
    #[serde(rename = "lifeLeechDamageTakenCount")]
    pub life_leech_damage_taken_count: i64,
    #[serde(rename = "downedDamageTaken")]
    pub downed_damage_taken: i64,
    #[serde(rename = "downedDamageTakenCount")]
    pub downed_damage_taken_count: i64,
    #[serde(rename = "damageBarrier")]
    pub damage_barrier: i64,
    #[serde(rename = "damageBarrierCount")]
    pub damage_barrier_count: i64,
    #[serde(rename = "breakbarDamageTaken", serialize_with = "ser_f64")]
    pub breakbar_damage_taken: f64,
    #[serde(rename = "breakbarDamageTakenCount")]
    pub breakbar_damage_taken_count: i64,
    #[serde(rename = "blockedCount")]
    pub blocked_count: i64,
    #[serde(rename = "evadedCount")]
    pub evaded_count: i64,
    #[serde(rename = "missedCount")]
    pub missed_count: i64,
    #[serde(rename = "dodgeCount")]
    pub dodge_count: i64,
    #[serde(rename = "invulnedCount")]
    pub invulned_count: i64,
    #[serde(rename = "interruptedCount")]
    pub interrupted_count: i64,
    #[serde(rename = "downCount")]
    pub down_count: i64,
    #[serde(rename = "downDuration")]
    pub down_duration: i64,
    #[serde(rename = "deadCount")]
    pub dead_count: i64,
    #[serde(rename = "deadDuration")]
    pub dead_duration: i64,
    #[serde(rename = "dcCount")]
    pub dc_count: i64,
    #[serde(rename = "dcDuration")]
    pub dc_duration: i64,
    #[serde(rename = "boonStrips")]
    pub boon_strips: i64,
    #[serde(rename = "boonStripsTime", serialize_with = "ser_f64")]
    pub boon_strips_time: f64,
    #[serde(rename = "conditionCleanses")]
    pub condition_cleanses: i64,
    #[serde(rename = "conditionCleansesTime", serialize_with = "ser_f64")]
    pub condition_cleanses_time: f64,
    #[serde(rename = "receivedCrowdControl")]
    pub received_crowd_control: i64,
    #[serde(rename = "receivedCrowdControlDuration", serialize_with = "ser_f64")]
    pub received_crowd_control_duration: f64,
    #[serde(rename = "stunBreak")]
    pub stun_break: i64,
    #[serde(rename = "removedStunDuration", serialize_with = "ser_f64")]
    pub removed_stun_duration: f64,
}

/// JsonPlayerSupport（14 键）。
#[derive(Serialize)]
pub struct JsonPlayerSupport {
    #[serde(rename = "resurrects")]
    pub resurrects: i64,
    #[serde(rename = "resurrectTime", serialize_with = "ser_f64")]
    pub resurrect_time: f64,
    #[serde(rename = "condiCleanse")]
    pub condi_cleanse: i64,
    #[serde(rename = "condiCleanseTime", serialize_with = "ser_f64")]
    pub condi_cleanse_time: f64,
    #[serde(rename = "condiCleanseSelf")]
    pub condi_cleanse_self: i64,
    #[serde(rename = "condiCleanseTimeSelf", serialize_with = "ser_f64")]
    pub condi_cleanse_time_self: f64,
    #[serde(rename = "boonStrips")]
    pub boon_strips: i64,
    #[serde(rename = "boonStripsTime", serialize_with = "ser_f64")]
    pub boon_strips_time: f64,
    #[serde(rename = "boonStripDownContribution")]
    pub boon_strip_down_contribution: i64,
    #[serde(rename = "boonStripDownContributionTime", serialize_with = "ser_f64")]
    pub boon_strip_down_contribution_time: f64,
    #[serde(rename = "stunBreak")]
    pub stun_break: i64,
    #[serde(rename = "removedStunDuration", serialize_with = "ser_f64")]
    pub removed_stun_duration: f64,
    #[serde(rename = "stunBreakSelf")]
    pub stun_break_self: i64,
    #[serde(rename = "removedStunSelfDuration", serialize_with = "ser_f64")]
    pub removed_stun_self_duration: f64,
}

/// JsonDamageDist（23 字段;downContribution 按值省略）。
#[derive(Serialize)]
pub struct JsonDamageDist {
    #[serde(rename = "totalDamage")]
    pub total_damage: i64,
    #[serde(rename = "totalBreakbarDamage", serialize_with = "ser_f64")]
    pub total_breakbar_damage: f64,
    #[serde(rename = "min")]
    pub min: i64,
    #[serde(rename = "max")]
    pub max: i64,
    #[serde(rename = "hits")]
    pub hits: i64,
    #[serde(rename = "connectedHits")]
    pub connected_hits: i64,
    #[serde(rename = "crit")]
    pub crit: i64,
    #[serde(rename = "glance")]
    pub glance: i64,
    #[serde(rename = "flank")]
    pub flank: i64,
    #[serde(rename = "againstMoving")]
    pub against_moving: i64,
    #[serde(rename = "missed")]
    pub missed: i64,
    #[serde(rename = "invulned")]
    pub invulned: i64,
    #[serde(rename = "interrupted")]
    pub interrupted: i64,
    #[serde(rename = "evaded")]
    pub evaded: i64,
    #[serde(rename = "blocked")]
    pub blocked: i64,
    #[serde(rename = "shieldDamage")]
    pub shield_damage: i64,
    #[serde(rename = "critDamage")]
    pub crit_damage: i64,
    #[serde(rename = "downContribution", skip_serializing_if = "Option::is_none")]
    pub down_contribution: Option<i64>,
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "indirectDamage")]
    pub indirect_damage: bool,
}

// ===== rotation / weapons / consumables / death recap =====
#[derive(Serialize)]
pub struct JsonRotation {
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "skills")]
    pub skills: Vec<JsonSkill>,
}

#[derive(Serialize)]
pub struct JsonSkill {
    #[serde(rename = "castTime")]
    pub cast_time: i64,
    #[serde(rename = "duration")]
    pub duration: i64,
    #[serde(rename = "timeGained")]
    pub time_gained: i64,
    #[serde(rename = "quickness", serialize_with = "ser_f64")]
    pub quickness: f64,
    #[serde(rename = "ignoreOnRotationRender", skip_serializing_if = "Option::is_none")]
    pub ignore_on_rotation_render: Option<bool>,
}

#[derive(Serialize)]
pub struct JsonWeaponSet {
    #[serde(rename = "weapons")]
    pub weapons: Vec<String>,
    #[serde(rename = "timeframe")]
    pub timeframe: Vec<i64>,
}

#[derive(Serialize)]
pub struct JsonConsumable {
    #[serde(rename = "stack")]
    pub stack: i64,
    #[serde(rename = "duration")]
    pub duration: i64,
    #[serde(rename = "time")]
    pub time: i64,
    #[serde(rename = "id")]
    pub id: i64,
}

#[derive(Serialize)]
pub struct JsonDeathRecap {
    #[serde(rename = "deathTime")]
    pub death_time: i64,
    #[serde(rename = "toDown", skip_serializing_if = "Option::is_none")]
    pub to_down: Option<Vec<JsonDeathRecapItem>>,
    #[serde(rename = "toKill", skip_serializing_if = "Option::is_none")]
    pub to_kill: Option<Vec<JsonDeathRecapItem>>,
}

#[derive(Serialize)]
pub struct JsonDeathRecapItem {
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "indirectDamage")]
    pub indirect_damage: bool,
    #[serde(rename = "src")]
    pub src: String,
    #[serde(rename = "damage")]
    pub damage: i64,
    #[serde(rename = "time")]
    pub time: i64,
}
