//! `CombatData` 最小聚合容器。
//!
//! P1 只做「事件列表 + 类别计数 + per-agent/per-id 计数骨架」；C#
//! `CombatData` 的多路冗余字典（per-AgentItem/per-SkillID 各桶 + 时间序列表、
//! CombatDataFetchers.cs 的 ~180 个查询方法）在 P2 与 AgentItem 链接一起实现。
//!
//! 组织对齐 C# `CombatData` 构造器（CombatData.cs:536-704）的分桶语义：
//! `events` 收 damage/buff/cast/status 行级事件；metadata（单例/列表/字典）
//! 收进 `metadata`（C# 的 `_metaDataEvents` 域，不进事件列表）。

use std::collections::BTreeMap;

use crate::events::{
    BuffInfoEventFields, CombatEvent, GuildEventFields, LanguageEventFields, LogNpcUpdateEventFields,
    MapChangeEventFields, MapIdEventFields, RewardEventFields, ShardEventFields,
    SkillInfoEventFields, SquadCombatEndEventFields, SquadCombatStartEventFields,
    TickEventFields, TickRateEventFields, UnsupportedEventKind,
    AttackTargetEventFields, ErrorEventFields, FractalScaleEventFields, Gw2BuildEventFields,
    InstanceStartEventFields, PointOfViewEventFields, WvWTeamsEventFields,
};

/// Metadata 分桶（C# `CombatData._metaDataEvents` + `_rewardEvents`）。
#[derive(Debug, Clone, Default)]
pub struct MetaDataBucket {
    // 单例（C# 域：只保留最后一次赋值）
    pub instance_start: Option<InstanceStartEventFields>,
    pub language: Option<LanguageEventFields>,
    pub gw2_build: Option<Gw2BuildEventFields>,
    pub shard: Option<ShardEventFields>,
    pub map_id: Option<MapIdEventFields>,
    pub point_of_view: Option<PointOfViewEventFields>,
    pub fractal_scale: Option<FractalScaleEventFields>,
    pub wvw_teams: Option<WvWTeamsEventFields>,
    /// C# `metaDataEvents.LogStartEvent`：首个（未被 LogEnd 顶替的）
    /// SquadCombatStart（CombatDataFactory AddStateChangeEvent:66-71）。
    pub log_start_event: Option<SquadCombatStartEventFields>,
    /// C# `metaDataEvents.LogEndEvent`：最后赋值的 SquadCombatEnd（:82）。
    pub log_end_event: Option<SquadCombatEndEventFields>,
    // 列表
    pub map_changes: Vec<MapChangeEventFields>,
    pub squad_combat_starts: Vec<SquadCombatStartEventFields>,
    pub squad_combat_ends: Vec<SquadCombatEndEventFields>,
    pub log_npc_updates: Vec<LogNpcUpdateEventFields>,
    pub rewards: Vec<RewardEventFields>,
    pub guilds: Vec<GuildEventFields>,
    pub attack_targets: Vec<AttackTargetEventFields>,
    pub tick_rates: Vec<TickRateEventFields>,
    pub ticks: Vec<TickEventFields>,
    pub errors: Vec<ErrorEventFields>,
    // 字典（BuffInfo/BuffFormula 行与 SkillInfo/SkillTiming 行按 id 合并）
    pub buff_info_by_id: BTreeMap<u32, BuffInfoEventFields>,
    pub skill_info_by_id: BTreeMap<u32, SkillInfoEventFields>,
    /// IDToGUID 行级事件（按 ContentLocal 分派的 Guid 变体；C# 侧进
    /// `metaDataEvents.*GUIDEventsBy*` 字典 —— P1 保留行级 + 计数，
    /// 反查索引 P2）。
    pub id_to_guid: Vec<CombatEvent>,
}

/// 按类别的事件计数（行级，含 metadata 单例/列表的全部产生行）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CategoryCounts {
    pub direct_health_damage: u64,
    pub non_direct_health_damage: u64,
    pub no_damage_health_damage: u64,
    pub breakbar_damage: u64,
    pub breakbar_recovery: u64,
    pub crowd_control: u64,
    pub stun_break: u64,
    pub buff_apply: u64,
    pub buff_extension: u64,
    pub buff_stack_active: u64,
    pub buff_stack_deactive: u64,
    pub buff_remove_single: u64,
    pub buff_remove_all: u64,
    pub buff_remove_manual: u64,
    pub animated_cast: u64,
    pub emote: u64,
    pub gadget_interact: u64,
    pub bundle_pick_up: u64,
    pub weapon_swap: u64,
    pub enter_combat: u64,
    pub exit_combat: u64,
    pub alive: u64,
    pub dead: u64,
    pub down: u64,
    pub spawn: u64,
    pub despawn: u64,
    pub health_update: u64,
    pub barrier_update: u64,
    pub max_health_update: u64,
    pub team_change: u64,
    pub targetable: u64,
    pub visibility: u64,
    pub breakbar_state: u64,
    pub breakbar_percent: u64,
    pub glider: u64,
    pub last90_before_down: u64,
    pub movement: u64,
    pub metadata_singletons: u64,
    pub metadata_list: u64,
    pub metadata_guid: u64,
    /// 被主分发消费但落在 C# `default` 丢弃分支的行（如伤害行的
    /// `Activation` 结果）或 RuleSet 等无输出 state change。
    pub dropped_by_cs: u64,
}

/// P1 战斗数据容器。索引骨架：`per_agent_counts` = 事件参与 agent 地址计数
/// （一事件至多记 2 个地址，对应 C# 按 Src/Dst 双路建索引的雏形）；
/// `per_id_counts` = 涉 skill/buff id 出现计数（cast/damage/buff 行）。
#[derive(Debug, Clone)]
pub struct CombatData {
    /// 行级语义事件（时间序由 `factory` 保证）。
    pub events: Vec<CombatEvent>,
    /// Metadata 分桶。
    pub metadata: MetaDataBucket,
    pub counts: CategoryCounts,
    /// P1 未类型化 state change 的显式计数（H 组 + C# 忽略行）。
    pub unsupported: BTreeMap<UnsupportedEventKind, u64>,
    /// 参与 agent 地址 → 参与事件数（骨架索引，P2 换成 Vec<索引>）。
    pub per_agent_counts: BTreeMap<u64, u64>,
    /// 涉技能/buff id → 出现次数。
    pub per_id_counts: BTreeMap<u32, u64>,
    /// arc 版本（事件化所依据的 build/revision）。
    pub arc_version: (i32, i32),
    /// 归零后日志时间轴（与 `EvtcRawLog` 一致）。
    pub log_start_offset: i64,
    pub log_end_time: i64,
}
