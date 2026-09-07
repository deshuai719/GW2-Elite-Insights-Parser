//! 类型化战斗事件与载荷结构。
//!
//! 对齐 `GW2EIEvtcParser/ParsedData/CombatEvents/` 的事件类层次（快照
//! `3b7278f9b`）。C# 用继承（`TimeCombatEvent ← SkillEvent ← …`），Rust 用
//! **组合**：组内公共字段放共享 struct（`StatusEventFields`/`SkillEventFields`/
//! `BuffEventFields`），具体事件 = 外层 struct 持有 base + 特有字段。
//!
//! P1 范围（新世代 ≥ 20260501 分发路径）：
//! - From/To/Caster 等持 `u64` **agent 地址**（P1 无 AgentItem；
//!   与 agent 区链接是 P2 `CompleteAgentsAndLogData` 的工作，地址 0 表示
//!   C# 的 `_unknownAgent`）。
//! - H 组（Effect/Marker/SquadMarker/Transformation/Missile/WvWObjective/
//!   GadgetCapture/GadgetAnimation）不做，分发记 `UnsupportedEventKind`
//!   计数（可观测，不静默丢弃）。
//!
//! 每个事件类型的文档注释标注对应 C# 文件与构造语义位置。

use gw2ei_parse::{AnimationStart, AnimationStop, BreakbarState, BuffStackType, Iff, LogType};

use crate::spec::Spec;

// ===== 共享字段组 =====

/// `StatusEvent`（StatusEvent.cs）：state change 状态事件的公共字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusEventFields {
    pub time: i64,
    /// agent 地址（`Src`）。
    pub src: u64,
}

/// `SkillEvent`（SkillEvent.cs）：涉技能事件的公共字段。
/// `AgainstDowned` 在 C# 由子类构造时设置，此处并入（P1 直填）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillEventFields {
    pub time: i64,
    pub from: u64,
    pub to: u64,
    /// 技能 id（arc 内置负 id 的位型；P2 链接 SkillItem）。
    pub skill_id: u32,
    pub iff: Iff,
    pub is_over_ninety: bool,
    pub against_under_fifty: bool,
    pub is_moving: bool,
    pub against_moving: bool,
    pub is_flanking: bool,
    pub against_downed: bool,
}

/// `BuffEvent`（BuffEvent.cs）：buff 事件公共字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffEventFields {
    pub time: i64,
    pub buff_id: u32,
    pub by: u64,
    pub to: u64,
    pub iff: Iff,
}

// ===== A 组：伤害事件（DamageEvents/）=====

/// `HealthDamageEvent`（HealthDamageEvent.cs）的字段面。三个具体事件
/// （Direct/NonDirect/NoDamage）在 C# 里字段集完全相同、仅构造时按
/// `DamageResult` 填值不同；Rust 用同一载荷 + 三个枚举变体区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthDamageEventFields {
    pub skill: SkillEventFields,
    pub health_damage: i32,
    pub shield_damage: i32,
    pub has_hit: bool,
    pub has_crit: bool,
    pub has_glanced: bool,
    /// `IsLifeLeech`（症状直伤行的 LifeLeech 语义，Direct 恒 false）。
    pub is_life_leech: bool,
    pub is_blind: bool,
    pub is_absorbed: bool,
    pub is_blocked: bool,
    pub is_evaded: bool,
    /// NoDamage 行（Interrupt/KillingBlow/Downed）标记。
    pub is_not_a_damage_event: bool,
    pub has_interrupted: bool,
    pub has_downed: bool,
    pub has_killed: bool,
}

impl Default for HealthDamageEventFields {
    fn default() -> Self {
        Self {
            skill: SkillEventFields {
                time: 0,
                from: 0,
                to: 0,
                skill_id: 0,
                iff: Iff::Unknown,
                is_over_ninety: false,
                against_under_fifty: false,
                is_moving: false,
                against_moving: false,
                is_flanking: false,
                against_downed: false,
            },
            health_damage: 0,
            shield_damage: 0,
            has_hit: false,
            has_crit: false,
            has_glanced: false,
            is_life_leech: false,
            is_blind: false,
            is_absorbed: false,
            is_blocked: false,
            is_evaded: false,
            is_not_a_damage_event: false,
            has_interrupted: false,
            has_downed: false,
            has_killed: false,
        }
    }
}

// ===== B 组：破蔑/控场/晕眩打断（NonDamageEvents/）=====

/// `BreakbarChangeEvent` 中间体的值：`round(Value/10.0, 1)`（C# 1 位小数）。
/// 按 skill/来源再分流为 BreakbarDamage 或 BreakbarRecovery。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreakbarEventFields {
    pub skill: SkillEventFields,
    /// `BreakbarDamage`/`BreakbarRecovered`（自然恢复为正值、软控压过自然
    /// 回复为负）。
    pub value: f64,
}

/// `CrowdControlEvent`：Duration=Value、DefianceCalculation=Value+Overstack。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrowdControlEventFields {
    pub skill: SkillEventFields,
    pub duration: i32,
    pub defiance_calculation: i32,
}

/// `StunBreakEvent`：方向修正 To=From(原 src)、From=unknown（StunBreakEvent.cs:12-13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StunBreakEventFields {
    pub skill: SkillEventFields,
    pub remaining_duration: i32,
}

// ===== C 组：buff apply / stack（BuffEvents/BuffApplies、BuffStacks）=====

/// `AbstractBuffApplyEvent`：BuffInstance=Pad、By=Src、To=Dst。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffApplyBaseFields {
    pub base: BuffEventFields,
    pub buff_instance: u32,
}

/// `BuffApplyEvent`：Initial=state==BuffInitial、AppliedDuration=Value；
/// Initial 且 build ≥ 20231107 时 OriginalAppliedDuration=BuffDmg 否则同
/// AppliedDuration（BuffApplyEvent.cs:25-34）。`added_active` =
/// `IsShields > 0`（BuffApplyEvent.cs:29 —— P3 buff 仿真入参，P1 未存）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffApplyEventFields {
    pub apply: BuffApplyBaseFields,
    pub initial: bool,
    pub applied_duration: i32,
    pub original_applied_duration: i32,
    /// C# `_addedActive`（IsShields>0）。
    pub added_active: bool,
}

/// `BuffExtensionEvent`：NewDuration=OverstackValue、ExtendedDuration=max(Value,0)
///（BuffExtensionEvent.cs:20-26；OffsetNewDuration 的二次修正属 P3 buff 模拟）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffExtensionEventFields {
    pub apply: BuffApplyBaseFields,
    pub new_duration: u32,
    pub extended_duration: i64,
}

/// `BuffStackEvent`（BuffStacks/BuffStackEvent.cs）：To=SrcAgent、By=unknown。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffStackEventFields {
    pub base: BuffEventFields,
    pub buff_instance: u32,
}

/// `BuffStackActiveEvent`：BuffInstance=(uint)DstAgent（BuffStackActiveEvent.cs:10-12）。
pub type BuffStackActiveEventFields = BuffStackEventFields;

/// `BuffStackDeactiveEvent`：BuffInstance=Pad、ResetToDuration=Value
///（BuffStackDeactiveEvent.cs:10-13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffStackDeactiveEventFields {
    pub stack: BuffStackEventFields,
    pub reset_to_duration: i32,
}

// ===== D 组：buff remove（BuffEvents/BuffRemoves）=====

/// `AbstractBuffRemoveEvent`：RemovedDuration=Value，**方向修正 By=DstAgent、
/// To=SrcAgent**（AbstractBuffRemoveEvent.cs:13-15）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffRemoveBaseFields {
    pub base: BuffEventFields,
    pub removed_duration: i32,
}

/// `BuffRemoveSingleEvent`：BuffInstance=Pad。`overstack_or_natural_end` =
/// IFF Unknown && 施放者 unknown && DstAgent==0（近似判定，见 factory）；
/// 由 agent 表精确判 `_byShouldntBeUnknown`（C# 侧为 DstAgent!=0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffRemoveSingleEventFields {
    pub remove: BuffRemoveBaseFields,
    pub buff_instance: u32,
    /// C# `OverstackOrNaturalEnd`（BuffRemoveSingleEvent.cs:13）。
    pub overstack_or_natural_end: bool,
}

/// `BuffRemoveAllEvent`：RemovedStacks=Result(raw)、末层时长=BuffDmg
///（BuffRemoveAllEvent.cs:14-16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffRemoveAllEventFields {
    pub remove: BuffRemoveBaseFields,
    pub removed_stacks: u8,
    pub last_removed_duration: i32,
}

/// `BuffRemoveManualEvent`：与 remove 基类同字段（无额外状态）。
pub type BuffRemoveManualEventFields = BuffRemoveBaseFields;

// ===== E 组：cast（CastEvents/）=====

/// `CastEvent.AnimationStatus`（CastEvent.cs:7）：P1 按 C# 构造端的赋值
/// （SetAcceleration/WeaponSwap 分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationStatus {
    Unknown,
    Reduced,
    Interrupted,
    Full,
    Instant,
}

/// `AnimatedCastEvent`（AnimatedCastEvent.cs）的公共字段面。配对语义在
/// factory 的 `CreateCastEvents`；此处只存结果值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatedCastEventFields {
    pub time: i64,
    pub skill_id: u32,
    pub caster: u64,
    /// `EffectTarget`：新世代 start 行的 DstAgent（非 0 时）；默认 unknown(0)。
    pub effect_target: u64,
    /// start 行 Result 字节解码（孤儿 end 时 None 语义 = 未设置）。
    pub anim_start: AnimationStart,
    /// end 行 Result 字节解码（孤儿 start 时未设置）。
    pub anim_stop: AnimationStop,
    pub status: AnimationStatus,
    pub saved_duration: i32,
    pub expected_duration: i32,
    pub actual_duration: i32,
    /// `Acceleration`：round(…, 3) 后的值。
    pub acceleration: f64,
}

/// `EmoteEvent`（EmoteEvent.cs）：EmoteID = start 行 AnimationStart 时
/// OverstackValue 否则 Pad。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmoteEventFields {
    pub base: AnimatedCastEventFields,
    pub emote_id: u64,
}

/// `GadgetInteractEvent`（Gadget/GadgetInteractEvent.cs）：Gadget=EffectTarget，
/// 无额外字段（构造分支对 status/saved/expected 的修正见 factory）。
pub type GadgetInteractEventFields = AnimatedCastEventFields;

/// `BundlePickUpEvent`（Gadget/BundlePickUpEvent.cs）：Caster 置 unknown(0)，
/// BundleID=start 行 OverstackValue。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BundlePickUpEventFields {
    pub base: AnimatedCastEventFields,
    pub bundle_id: u64,
}

/// `WeaponSwapEvent`（WeaponSwapEvent.cs）：SwappedTo=(int)DstAgent、
/// SwappedFrom=Value（build ≥ 20240627 才有，旧 build 为 -1 —— P1 世代恒有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponSwapEventFields {
    pub time: i64,
    pub caster: u64,
    pub swapped_to: i32,
    pub swapped_from: i32,
}

// ===== F 组：状态事件（StatusEvents/）=====

/// `CombatStatusEvent`（Enter/ExitCombat）：Subgroup=DstAgent、
/// Spec=GetSpec(Value, BuffDmg)（CombatStatusEvent.cs:10-13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatStatusEventFields {
    pub base: StatusEventFields,
    pub subgroup: u64,
    pub spec: Spec,
    pub base_spec: Spec,
}

/// `HealthUpdateEvent`/`BarrierUpdateEvent`：percent = round(DstAgent/100.0, 2)；
/// HealthUpdate 超 100 压到 100、BarrierUpdate 超 100 置 0（HealthUpdateEvent.cs:15-17 /
/// BarrierUpdateEvent.cs:12-15 —— 两处对超界处理不同，C# 原样）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HealthPercentEventFields {
    pub base: StatusEventFields,
    pub percent: f64,
}

/// `MaxHealthUpdateEvent`：MaxHealth=(int)DstAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxHealthEventFields {
    pub base: StatusEventFields,
    pub max_health: i32,
}

/// `TeamChangeEvent`：TeamIDInto=DstAgent、TeamIDComingFrom=Value
///（build ≥ 20240612 才有 ComingFrom；旧 build 为 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamChangeEventFields {
    pub base: StatusEventFields,
    pub team_id_into: u64,
    pub team_id_coming_from: u64,
}

/// `TargetableEvent`：Targetable = DstAgent==1。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetableEventFields {
    pub base: StatusEventFields,
    pub targetable: bool,
}

/// `VisibilityEvent`：由 Targetable(门控世代)/StealthChange 行构造；
/// Visible = Targetable 行 ? Value==1 : DstAgent==1；Gadget 字段保留
///（VisibilityEvent.cs:10-14，C# 的 GadgetNameVisible 来源 P1 不做）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityEventFields {
    pub base: StatusEventFields,
    pub visible: bool,
}

/// `BreakbarStateEvent`：State=GetBreakbarState(Value)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakbarStateEventFields {
    pub base: StatusEventFields,
    pub state: BreakbarState,
}

/// `BreakbarPercentEvent`：percent = round(100 × f32_bits(Value), 2)，>100 压 100。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreakbarPercentEventFields {
    pub base: StatusEventFields,
    pub percent: f64,
}

/// `GliderEvent`：GliderDeployed = Value==1。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GliderEventFields {
    pub base: StatusEventFields,
    pub deployed: bool,
}

/// `Last90BeforeDownEvent`：TimeSinceLast90=(long)DstAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Last90BeforeDownEventFields {
    pub base: StatusEventFields,
    pub time_since_last_90: u64,
}

/// `MovementEvent`（Position/Rotation/Velocity/Teleport）：坐标由
/// DstAgent 低 8B（两个 f32，x/y）+ Value 位型 f32（z）解包
///（MovementEvent.cs 的 UnpackMovementData）。解包后非有限值（NaN/Inf）
/// 在构造时丢弃 —— 该丢弃在 C# 位于 replay 消费端（PositionEvent.cs 等
/// AddPoint3D），事件层不丢；P1 无 replay 消费端，丢弃提前到构造并记录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementEventFields {
    pub base: StatusEventFields,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// `JumpEvent`（占位，Jump state change 被 IsValid ① 过滤、实际不可达；
/// 类型保留供未来世代启用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpEventFields {
    pub base: StatusEventFields,
    pub on_land: bool,
}

// ===== H 组子集：Effect（StatusEvents/EffectEvents；P3b 最小面）=====

/// Effect 事件的 P3b 消费最小面（EffectEventCBTS51/CBTS45 构造语义 +
/// EffectEnd 配对）。
///
/// C# `EffectEvent`（EffectEvent.cs）还带 Orientation/Scale/Flags 等
/// combat-replay 消费字段，P3b 不做；位置仅地面 effect（`IsAroundDst`
/// false）解包（NonSplitEffectEvent.cs:9-16 的 Value/BuffDmg/Overstack
/// 三 f32）。`end_time` = EffectEndEventCBTS51 按 TrackingID 配对（
/// EffectEvent.SetDynamicEndTime，仅写一次）—— Sand Shade 合成与部分
/// Effect finder checker（MineDetonation 等）消费。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectEventFields {
    pub time: i64,
    pub src: u64,
    /// 非 0 = `IsAroundDst`（跟随该 agent；CBTS51 DstAgent != 0）。
    pub dst: u64,
    /// arc effect id（C# `EffectID` = SkillID 位型，与 IDToGUID 的
    /// ContentID 同源）。
    pub effect_id: u32,
    /// 事件化修正后的时长：CBTS51 行值（Duration==0 时按 GUID 事件
    /// DefaultDuration 兜底）；CBTS45 恒 0。
    pub duration: i64,
    /// EffectEnd 配对后的动态结束时间（`DynamicEndTime`）。
    pub end_time: Option<i64>,
    /// CBTS51 的 TrackingID（u32 由 IsBuffRemove/Ninety/Fifty/Moving 四字节
    /// 拼出）；CBTS45 无 tracking（恒 0）。
    pub tracking_id: u32,
    /// CBTS45 世代标记（SkillID==0 的 end 行 CBTS45 直接丢弃；Sand Shade
    /// 合成跳过 CBTS45 —— ScourgeHelper.cs:60-62）。
    pub cbts45: bool,
    /// 位置（DstAgent==0 时；C# `Position`）。消费端 checker 的
    /// 位置比较用（UsingNoSecondaryEffect…OnSamePosition 等）。
    pub position: (f32, f32, f32),
}

/// `MissileEvent`（MissileEvents/MissileEvent.cs）的 P3b 最小面：
/// MissileCreate 行(Src、SkillID、Time)。Launch/Remove 配对与
/// DidHit 等消费端(P3c+)需要时扩展。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissileEventFields {
    pub time: i64,
    /// 施放者 agent 地址。
    pub src: u64,
    pub skill_id: u32,
}

/// IDToGUID Effect 行解析出的 effect 元数据（C# `EffectGUIDEvent`；
/// EffectGUIDEvent.cs:12-23）。GUID hex 32 大写字符（GuidEventFields::guid_hex
/// 同一编码）；`default_duration` 在 arc build ≥ ExtraDataInGUIDEvents
/// (20241030) 时由 BuffDmg f32 位型给出（毫秒值以 float 承载），否则 -1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectGuidInfo {
    pub guid_hex: String,
    pub default_duration: i64,
}

// ===== H 组子集：WvWObjectiveStatus（P4；StatusEvents/WvWObjectiveStatusEvent.cs）=====

/// `WvWObjectiveStatusEvent`（WvWObjectiveStatusEvent.cs）的**聚合面**：
/// 每行 `MapID=Value`、`ObjectiveID=(int)SkillID`、`AutoUpgradeProgress=Pad`，
/// owner = (TeamID=(uint)BuffDmg, Time)。同 key 行在工厂按首现序聚合进同一
/// 事件（owners 追加不合并）。行级构造在 C# 构造时即按静态表判 `IsUnknown`
/// 丢弃 —— 表过滤留在 JSON 层（gw2ei-json/wvw.rs），模型层保原始聚合
/// （IsUnknown 是 (map, objective) 的静态属性，后置过滤结果与 C# 等价）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WvWObjectiveStatusEventFields {
    /// 地图 id（行 Value）。
    pub map_id: i32,
    /// objective id（行 SkillID 的 i32 视图）。
    pub objective_id: i32,
    /// 升级进度（行 Pad）。
    pub auto_upgrade_progress: u32,
    /// (team id, time) 对，按行事件时间序追加。
    pub owners: Vec<(u32, i64)>,
}

impl WvWObjectiveStatusEventFields {
    /// 聚合 key（C# `(MapID << 16) + ObjectiveID`，long 运算）。
    pub fn key(&self) -> i64 {
        (i64::from(self.map_id) << 16) + i64::from(self.objective_id)
    }
}

// ===== G 组：metadata 事件（MetaDataEvents/）=====

/// `InstanceStartEvent`：TimeOffsetFromInstanceCreation=logStart-SrcAgent、
/// InstanceIP 由 Value 的 4 字节拼 IPv4（Value==0 无）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceStartEventFields {
    pub time: i64,
    pub time_offset_from_instance_creation: i64,
    /// `InstanceIP` 无时为空（C# string? null）。
    pub instance_ip: Option<[u8; 4]>,
}

/// `LanguageEvent`：Language=GetLanguage((byte)SrcAgent)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageEventFields {
    pub language: gw2ei_parse::Language,
}

/// `GW2BuildEvent`：Build=SrcAgent（非 0 才构造，AddStateChangeEvent 门控）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gw2BuildEventFields {
    pub build: u64,
}

/// `ShardEvent`：ShardID=SrcAgent、UpperShardID=DstAgent、UserWorldID0/1=
/// Value/BuffDmg；Region 本地判定（UserWorldID0>0 时），否则 P1 Unknown
///（C# 走 map API，P2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardEventFields {
    pub shard_id: u64,
    pub upper_shard_id: u64,
    pub user_world_id_0: u32,
    pub user_world_id_1: u32,
    /// `RegionEnum` 本地可判定子集。
    pub region: Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Na,
    Eu,
    Cn,
    Unknown,
}

impl Region {
    /// `ShardEvent.GetRegion`（ShardEvent.cs:12-24）。
    pub fn from_shard_id(shard_id: u64) -> Region {
        if (1000..2000).contains(&shard_id) {
            Region::Na
        } else if (2000..3000).contains(&shard_id) {
            Region::Eu
        } else if (7000..8000).contains(&shard_id) {
            Region::Cn
        } else {
            Region::Unknown
        }
    }
}

/// `MapIDEvent`：MapID=(int)SrcAgent、MapType=DstAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapIdEventFields {
    pub time: i64,
    pub map_id: i32,
    pub map_type: u64,
}

/// `MapChangeEvent`：同 MapIDEvent + OldMapID=(int)DstAgent、MapType=Value。
/// C# 里 MapChange 的 MapID 字段 = SrcAgent 低位（MapChangeEvent.cs:8-12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapChangeEventFields {
    pub time: i64,
    pub map_id: i32,
    pub old_map_id: i32,
    pub map_type: i32,
}

/// `PointOfViewEvent`：PoV=SrcAgent（AnonymousPlayers 门控在设置层，P1 不过滤）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointOfViewEventFields {
    pub time: i64,
    pub pov: u64,
}

/// `FractalScaleEvent`：Scale=(byte)SrcAgent（SrcAgent==0 不构造）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FractalScaleEventFields {
    pub scale: u8,
}

/// `WvWTeamsEvent`：24B = Src(8) Dst(8) Value(4) BuffDmg(4)，按 6 个 u32
/// 拆 Red/Blue/Green 的 ShardID 与 TeamID（WvWTeamsEvent.cs:18-37）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WvWTeamsEventFields {
    pub red_shard_id: u32,
    pub blue_shard_id: u32,
    pub green_shard_id: u32,
    pub red_team_id: u32,
    pub blue_team_id: u32,
    pub green_team_id: u32,
}

/// `TickRateEvent`：TickRate=SrcAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickRateEventFields {
    pub time: i64,
    pub tick_rate: u64,
}

/// `TickEvent`：TickInterpolated=SrcAgent、TickSinceLastUpdate=DstAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickEventFields {
    pub time: i64,
    pub tick_interpolated: u64,
    pub tick_since_last_update: u64,
}

/// `ErrorEvent`：Message = 前 32B（Time/Src/Dst/Value/BuffDmg 回写）UTF-8。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorEventFields {
    pub message: String,
}

/// `LogDateEvent` 系公共字段（SquadCombatStart/End、LogNPCUpdate）：
/// ServerUnixTimeStamp=Value、LocalUnixTimeStamp=BuffDmg。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogDateEventFields {
    pub time: i64,
    pub server_unix_time_stamp: u32,
    pub local_unix_time_stamp: u32,
}

/// `SquadCombatStartEvent`：LogType=GetLogType((int)DstAgent)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SquadCombatStartEventFields {
    pub date: LogDateEventFields,
    pub log_type: LogType,
}

/// `SquadCombatEndEvent`：ByPoVMapExit=(DstAgent&1)==1。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SquadCombatEndEventFields {
    pub date: LogDateEventFields,
    pub by_pov_map_exit: bool,
}

/// `LogNPCUpdateEvent`：AgentID=(ushort)SrcAgent；TriggerAgent 由 DstAgent
/// 地址承接（DstAgent==0 无），TriggerIsGadget=IsFlanking>0。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogNpcUpdateEventFields {
    pub date: LogDateEventFields,
    pub agent_id: u16,
    pub trigger_agent: u64,
    pub trigger_is_gadget: bool,
}

/// `RewardEvent`：RewardID=DstAgent、RewardType=Value。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardEventFields {
    pub time: i64,
    pub reward_id: u64,
    pub reward_type: i32,
}

/// `GuildEvent`：GuildKey = 字节交换拼接的 32 位十六进制串（8-4-4-4-12），
/// 见 `GuildEvent.guid_hex` 与工厂（GuildEvent.cs:12-53）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildEventFields {
    pub src: u64,
    pub guild_key_hex: String,
}

/// `AttackTargetEvent`：AttackTarget=SrcAgent、Src=DstAgent（字段互换！
/// AttackTargetEvent.cs:13-15）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackTargetEventFields {
    pub time: i64,
    /// 发出攻击目标更新的 agent（C# `Src`，来自 DstAgent）。
    pub src: u64,
    /// 被选为攻击目标的 agent（C# `AttackTarget`，来自 SrcAgent）。
    pub attack_target: u64,
    /// Targetable = Value==1。
    pub targetable: bool,
}

/// `IDToGUIDEvent` 族公共字段（IDToGUID/IDToGUIDEvent.cs）：GUID 由
/// SrcAgent + DstAgent 各 8B 拼成（C# `GUID(first8, last8)`，内存序即
/// 小端字节序），ContentID=SkillID。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidEventFields {
    pub guid_first8: u64,
    pub guid_last8: u64,
    pub content_id: u32,
}

impl GuidEventFields {
    /// C# `GUID.ToHex()`：16B 内存序大写 hex（Guid.cs:55-59）。
    pub fn guid_hex(&self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(32);
        for b in self
            .guid_first8
            .to_le_bytes()
            .iter()
            .chain(self.guid_last8.to_le_bytes().iter())
        {
            let _ = write!(out, "{b:02X}");
        }
        out
    }
}

/// `BuffInfoEvent` 的 BuffInfo 行字段（BuffInfoEvent.cs BuildFromBuffInfo:39-60）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffInfoRowFields {
    pub probably_invul: bool,
    pub probably_invert: bool,
    pub category_byte: u8,
    pub stacking_type_byte: u8,
    pub stacking_type: BuffStackType,
    pub probably_resistance: bool,
    pub max_stacks: u16,
    pub duration_cap: u32,
}

/// `BuffFormula` 行拆出的原始字段（BuffFormula.cs:74-127 的字段面；
/// `Attr1/Attr2` 的 `GetBuffAttribute` 语义与 `SortKey`/描述文本留 P2）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuffFormulaRowFields {
    /// `Type`（公式类型，i32 位型 → 作为 i32 使用）。
    pub formula_type: i32,
    pub byte_attr1: u8,
    pub byte_attr2: u8,
    pub constant_offset: f32,
    pub level_offset: f32,
    pub variable: f32,
    pub trait_src: i32,
    pub trait_self: i32,
    pub content_reference: f32,
    pub buff_src: i32,
    pub buff_self: i32,
    /// `ExtraNumber`/`ExtraNumberState`（Pad1 状态字节）。
    pub extra_number: u32,
    pub extra_number_state: u8,
    /// Npc/Player/Break 门控（BuffFormula.cs:74-76）。
    pub npc: bool,
    pub player: bool,
    pub is_break: bool,
}

/// `BuffInfoEvent` 聚合（BuffInfo 行 + BuffFormula 行合并）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuffInfoEventFields {
    pub buff_id: u32,
    pub info: Option<BuffInfoRowFields>,
    pub formulas: Vec<BuffFormulaRowFields>,
}

/// `SkillTiming` 行（SkillTiming.cs）：ActionByte=SrcAgent 低字节、
/// AtMillisecond=DstAgent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillTimingRowFields {
    pub action_byte: u8,
    pub action: gw2ei_parse::SkillAction,
    pub at_millisecond: u64,
}

/// `SkillInfoEvent` 聚合（SkillInfo 行 + SkillTiming 行合并）。SkillInfo 行
/// 的 Recharge/Range0/Range1/TooltipTime 由 Time(8B)+SrcAgent(8B) 按 4 个
/// f32 拆出（SkillInfoEvent.cs:35-46）。
#[derive(Debug, Clone, PartialEq)]
pub struct SkillInfoEventFields {
    pub skill_id: u32,
    pub recharge: f32,
    pub range0: f32,
    pub range1: f32,
    pub tooltip_time: f32,
    pub timings: Vec<SkillTimingRowFields>,
}

// ===== 事件分发容器 =====

/// 类型化战斗事件（扁平 enum）。P1 语义化后的字段面，分类同
/// `research/ei-combat-events.md` §1 的 A-G 组。
#[derive(Debug, Clone, PartialEq)]
pub enum CombatEvent {
    // A 直接/症状/无伤害行（Combat+IsBuff==0/!=0 与 DamageResult 分流）
    DirectHealthDamage(HealthDamageEventFields),
    NonDirectHealthDamage(HealthDamageEventFields),
    NoDamageHealthDamage(HealthDamageEventFields),
    // B 破蔑/控场/晕眩打断
    BreakbarDamage(BreakbarEventFields),
    BreakbarRecovery(BreakbarEventFields),
    CrowdControl(CrowdControlEventFields),
    StunBreak(StunBreakEventFields),
    // C buff apply / stack
    BuffApply(BuffApplyEventFields),
    BuffExtension(BuffExtensionEventFields),
    BuffStackActive(BuffStackActiveEventFields),
    BuffStackDeactive(BuffStackDeactiveEventFields),
    // D buff remove
    BuffRemoveSingle(BuffRemoveSingleEventFields),
    BuffRemoveAll(BuffRemoveAllEventFields),
    BuffRemoveManual(BuffRemoveManualEventFields),
    // E cast
    AnimatedCast(AnimatedCastEventFields),
    Emote(EmoteEventFields),
    GadgetInteract(GadgetInteractEventFields),
    BundlePickUp(BundlePickUpEventFields),
    WeaponSwap(WeaponSwapEventFields),
    // F 状态
    EnterCombat(CombatStatusEventFields),
    ExitCombat(CombatStatusEventFields),
    Alive(StatusEventFields),
    Dead(StatusEventFields),
    Down(StatusEventFields),
    Spawn(StatusEventFields),
    Despawn(StatusEventFields),
    HealthUpdate(HealthPercentEventFields),
    BarrierUpdate(HealthPercentEventFields),
    MaxHealthUpdate(MaxHealthEventFields),
    TeamChange(TeamChangeEventFields),
    Targetable(TargetableEventFields),
    Visibility(VisibilityEventFields),
    BreakbarState(BreakbarStateEventFields),
    BreakbarPercent(BreakbarPercentEventFields),
    Glider(GliderEventFields),
    Last90BeforeDown(Last90BeforeDownEventFields),
    Position(MovementEventFields),
    Rotation(MovementEventFields),
    Velocity(MovementEventFields),
    Teleport(MovementEventFields),
    Jump(JumpEventFields),
    // H 组子集：Effect（P3b；EffectEnd 行不产生事件只配对）
    Effect(EffectEventFields),
    // H 组子集：Missile（P3b 仅 MissileCreate 行）
    Missile(MissileEventFields),
    // G metadata —— 单例
    InstanceStart(InstanceStartEventFields),
    Language(LanguageEventFields),
    Gw2Build(Gw2BuildEventFields),
    Shard(ShardEventFields),
    MapId(MapIdEventFields),
    MapChange(MapChangeEventFields),
    PointOfView(PointOfViewEventFields),
    FractalScale(FractalScaleEventFields),
    WvWTeams(WvWTeamsEventFields),
    TickRate(TickRateEventFields),
    Tick(TickEventFields),
    ErrorEvent(ErrorEventFields),
    // G metadata —— 列表
    SquadCombatStart(SquadCombatStartEventFields),
    SquadCombatEnd(SquadCombatEndEventFields),
    LogNpcUpdate(LogNpcUpdateEventFields),
    Reward(RewardEventFields),
    Guild(GuildEventFields),
    AttackTarget(AttackTargetEventFields),
    // G metadata —— GUID 事件（IDToGUID 按 ContentLocal 分派）
    GuidEffect(GuidEventFields),
    GuidMarker(GuidEventFields),
    GuidSkill(GuidEventFields),
    GuidSpecies(GuidEventFields),
    GuidTeam(GuidEventFields),
    GuidEmote(GuidEventFields),
    GuidTransformation(GuidEventFields),
}

/// 事件时间访问 trait：所有载荷 struct 都能给出自身时间（无 Option——
/// 有时间的类型）。metadata 单例类（无时间）不实现。
pub trait EventTime {
    fn event_time(&self) -> i64;
}

macro_rules! impl_event_time {
    ($($t:ty => $self:ident, $e:expr),+ $(,)?) => {
        $( impl EventTime for $t {
            fn event_time(&self) -> i64 {
                let $self = self;
                $e
            }
        } )+
    };
}

impl_event_time! {
    HealthDamageEventFields => s, s.skill.time,
    StatusEventFields => s, s.time,
    BreakbarEventFields => s, s.skill.time,
    CrowdControlEventFields => s, s.skill.time,
    StunBreakEventFields => s, s.skill.time,
    BuffApplyEventFields => s, s.apply.base.time,
    BuffExtensionEventFields => s, s.apply.base.time,
    BuffStackEventFields => s, s.base.time,
    BuffStackDeactiveEventFields => s, s.stack.base.time,
    BuffRemoveBaseFields => s, s.base.time,
    BuffRemoveSingleEventFields => s, s.remove.base.time,
    BuffRemoveAllEventFields => s, s.remove.base.time,
    AnimatedCastEventFields => s, s.time,
    EmoteEventFields => s, s.base.time,
    BundlePickUpEventFields => s, s.base.time,
    WeaponSwapEventFields => s, s.time,
    CombatStatusEventFields => s, s.base.time,
    HealthPercentEventFields => s, s.base.time,
    MaxHealthEventFields => s, s.base.time,
    TeamChangeEventFields => s, s.base.time,
    TargetableEventFields => s, s.base.time,
    VisibilityEventFields => s, s.base.time,
    BreakbarStateEventFields => s, s.base.time,
    BreakbarPercentEventFields => s, s.base.time,
    GliderEventFields => s, s.base.time,
    Last90BeforeDownEventFields => s, s.base.time,
    MovementEventFields => s, s.base.time,
    JumpEventFields => s, s.base.time,
    InstanceStartEventFields => s, s.time,
    MapIdEventFields => s, s.time,
    MapChangeEventFields => s, s.time,
    PointOfViewEventFields => s, s.time,
    TickRateEventFields => s, s.time,
    TickEventFields => s, s.time,
    LogDateEventFields => s, s.time,
    SquadCombatStartEventFields => s, s.date.time,
    SquadCombatEndEventFields => s, s.date.time,
    LogNpcUpdateEventFields => s, s.date.time,
    RewardEventFields => s, s.time,
    AttackTargetEventFields => s, s.time,
}

impl CombatEvent {
    /// 事件时间（metadata 单例类无时间 —— C# `MetaDataEvent` 无 Time）。
    pub fn time(&self) -> Option<i64> {
        use CombatEvent::*;
        use EventTime as _;
        Some(match self {
            DirectHealthDamage(e) => e.event_time(),
            NonDirectHealthDamage(e) => e.event_time(),
            NoDamageHealthDamage(e) => e.event_time(),
            BreakbarDamage(e) => e.event_time(),
            BreakbarRecovery(e) => e.event_time(),
            CrowdControl(e) => e.event_time(),
            StunBreak(e) => e.event_time(),
            BuffApply(e) => e.event_time(),
            BuffExtension(e) => e.event_time(),
            BuffStackActive(e) => e.event_time(),
            BuffStackDeactive(e) => e.event_time(),
            BuffRemoveSingle(e) => e.event_time(),
            BuffRemoveAll(e) => e.event_time(),
            BuffRemoveManual(e) => e.event_time(),
            AnimatedCast(e) => e.event_time(),
            Emote(e) => e.event_time(),
            GadgetInteract(e) => e.event_time(),
            BundlePickUp(e) => e.event_time(),
            WeaponSwap(e) => e.event_time(),
            EnterCombat(e) => e.event_time(),
            ExitCombat(e) => e.event_time(),
            Alive(e) => e.event_time(),
            Dead(e) => e.event_time(),
            Down(e) => e.event_time(),
            Spawn(e) => e.event_time(),
            Despawn(e) => e.event_time(),
            HealthUpdate(e) => e.event_time(),
            BarrierUpdate(e) => e.event_time(),
            MaxHealthUpdate(e) => e.event_time(),
            TeamChange(e) => e.event_time(),
            Targetable(e) => e.event_time(),
            Visibility(e) => e.event_time(),
            BreakbarState(e) => e.event_time(),
            BreakbarPercent(e) => e.event_time(),
            Glider(e) => e.event_time(),
            Last90BeforeDown(e) => e.event_time(),
            Position(e) => e.event_time(),
            Rotation(e) => e.event_time(),
            Velocity(e) => e.event_time(),
            Teleport(e) => e.event_time(),
            Jump(e) => e.event_time(),
            Effect(e) => e.time,
            Missile(e) => e.time,
            InstanceStart(e) => e.event_time(),
            MapId(e) => e.event_time(),
            MapChange(e) => e.event_time(),
            PointOfView(e) => e.event_time(),
            TickRate(e) => e.event_time(),
            Tick(e) => e.event_time(),
            SquadCombatStart(e) => e.event_time(),
            SquadCombatEnd(e) => e.event_time(),
            LogNpcUpdate(e) => e.event_time(),
            Reward(e) => e.event_time(),
            AttackTarget(e) => e.event_time(),
            // 无时间 metadata 单例 / 字典行
            Language(_) | Gw2Build(_) | Shard(_) | FractalScale(_) | WvWTeams(_)
            | ErrorEvent(_) | Guild(_) | GuidEffect(_) | GuidMarker(_) | GuidSkill(_)
            | GuidSpecies(_) | GuidTeam(_) | GuidEmote(_) | GuidTransformation(_) => {
                return None;
            }
        })
    }
}

/// P1 分发显式标记：H 组与 C# 明确忽略的行。工厂遇这些 state change
/// 计数（不产生事件、不静默丢弃），字段语义留后续阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnsupportedEventKind {
    /// Effect_45/Effect_51/EffectGround*/EffectAgent*（成对生命周期组）。
    Effect,
    /// Marker（含 NewMarkerEventBehavior 世代差异）。
    Marker,
    /// SquadMarker。
    SquadMarker,
    /// Transformation。
    Transformation,
    /// MissileCreate/Launch/Remove。
    Missile,
    /// WvWObjectiveStatus（多行聚合）。
    WvWObjectiveStatus,
    /// GadgetCaptureOutlineShow/SplitPercent/Hide/Point（聚合）。
    GadgetCapture,
    /// GadgetAnimation（Next 链）。
    GadgetAnimation,
    /// C# "Ignore for now" 行（GadgetNameVisible/EffectMissileCreate/RuleSet）。
    IgnoredByCs,
    /// 已注册扩展（HealingStats）的行级事件化 —— P3，P1 只计数。
    Extension,
}

impl UnsupportedEventKind {
    pub fn label(self) -> &'static str {
        match self {
            UnsupportedEventKind::Effect => "Effect",
            UnsupportedEventKind::Marker => "Marker",
            UnsupportedEventKind::SquadMarker => "SquadMarker",
            UnsupportedEventKind::Transformation => "Transformation",
            UnsupportedEventKind::Missile => "Missile",
            UnsupportedEventKind::WvWObjectiveStatus => "WvWObjectiveStatus",
            UnsupportedEventKind::GadgetCapture => "GadgetCapture",
            UnsupportedEventKind::GadgetAnimation => "GadgetAnimation",
            UnsupportedEventKind::IgnoredByCs => "IgnoredByCs",
            UnsupportedEventKind::Extension => "Extension",
        }
    }
}

impl CombatEvent {
    /// 事件参与 agent 地址（第一个 = 来源侧/施放者侧；第二个 = 目标侧/第二
    /// 参与者；无则为 0）。对应 C# 构造器的 Src/Dst 双路索引雏形。
    /// 映射以各载荷字段的「agent 承载」为准（buff remove 的方向修正后
    /// By/To 已交换；buff stack 事件 to=src 侧）。
    pub fn participants(&self) -> (u64, u64) {
        use CombatEvent::*;
        match self {
            DirectHealthDamage(e) => (e.skill.from, e.skill.to),
            NonDirectHealthDamage(e) => (e.skill.from, e.skill.to),
            NoDamageHealthDamage(e) => (e.skill.from, e.skill.to),
            BreakbarDamage(e) => (e.skill.from, e.skill.to),
            BreakbarRecovery(e) => (e.skill.from, e.skill.to),
            CrowdControl(e) => (e.skill.from, e.skill.to),
            StunBreak(e) => (e.skill.from, e.skill.to),
            BuffApply(e) => (e.apply.base.by, e.apply.base.to),
            BuffExtension(e) => (e.apply.base.by, e.apply.base.to),
            BuffStackActive(e) => (e.base.to, e.base.by),
            BuffStackDeactive(e) => (e.stack.base.to, e.stack.base.by),
            BuffRemoveSingle(e) => (e.remove.base.by, e.remove.base.to),
            BuffRemoveAll(e) => (e.remove.base.by, e.remove.base.to),
            BuffRemoveManual(e) => (e.base.by, e.base.to),
            AnimatedCast(e) => (e.caster, e.effect_target),
            GadgetInteract(e) => (e.caster, e.effect_target),
            Emote(e) => (e.base.caster, e.base.effect_target),
            BundlePickUp(e) => (e.base.caster, e.base.effect_target),
            WeaponSwap(e) => (e.caster, 0),
            EnterCombat(e) => (e.base.src, 0),
            ExitCombat(e) => (e.base.src, 0),
            Alive(e) => (e.src, 0),
            Dead(e) => (e.src, 0),
            Down(e) => (e.src, 0),
            Spawn(e) => (e.src, 0),
            Despawn(e) => (e.src, 0),
            HealthUpdate(e) => (e.base.src, 0),
            BarrierUpdate(e) => (e.base.src, 0),
            BreakbarPercent(e) => (e.base.src, 0),
            MaxHealthUpdate(e) => (e.base.src, 0),
            TeamChange(e) => (e.base.src, 0),
            Targetable(e) => (e.base.src, 0),
            Visibility(e) => (e.base.src, 0),
            BreakbarState(e) => (e.base.src, 0),
            Glider(e) => (e.base.src, 0),
            Last90BeforeDown(e) => (e.base.src, 0),
            Jump(e) => (e.base.src, 0),
            Position(e) => (e.base.src, 0),
            Rotation(e) => (e.base.src, 0),
            Velocity(e) => (e.base.src, 0),
            Teleport(e) => (e.base.src, 0),
            Effect(e) => (e.src, e.dst),
            Missile(e) => (e.src, 0),
            InstanceStart(_) => (0, 0),
            Language(_) => (0, 0),
            Gw2Build(_) => (0, 0),
            Shard(_) => (0, 0),
            MapId(_) => (0, 0),
            MapChange(_) => (0, 0),
            FractalScale(_) => (0, 0),
            WvWTeams(_) => (0, 0),
            TickRate(_) => (0, 0),
            Tick(_) => (0, 0),
            ErrorEvent(_) => (0, 0),
            SquadCombatStart(_) => (0, 0),
            SquadCombatEnd(_) => (0, 0),
            LogNpcUpdate(_) => (0, 0),
            Reward(_) => (0, 0),
            GuidEffect(_) => (0, 0),
            GuidMarker(_) => (0, 0),
            GuidSkill(_) => (0, 0),
            GuidSpecies(_) => (0, 0),
            GuidTeam(_) => (0, 0),
            GuidEmote(_) => (0, 0),
            GuidTransformation(_) => (0, 0),
            PointOfView(e) => (e.pov, 0),
            Guild(e) => (e.src, 0),
            AttackTarget(e) => (e.src, e.attack_target),
        }
    }

    /// 事件相关的 skill/buff id（SkillEvent 系 / cast 系；buff 事件取
    /// buff_id）。用于 per-id 计数骨架。
    pub fn related_id(&self) -> Option<u32> {
        use CombatEvent::*;
        match self {
            DirectHealthDamage(e) => Some(e.skill.skill_id),
            NonDirectHealthDamage(e) => Some(e.skill.skill_id),
            NoDamageHealthDamage(e) => Some(e.skill.skill_id),
            BreakbarDamage(e) => Some(e.skill.skill_id),
            BreakbarRecovery(e) => Some(e.skill.skill_id),
            CrowdControl(e) => Some(e.skill.skill_id),
            StunBreak(e) => Some(e.skill.skill_id),
            BuffApply(e) => Some(e.apply.base.buff_id),
            BuffExtension(e) => Some(e.apply.base.buff_id),
            BuffStackActive(e) => Some(e.base.buff_id),
            BuffStackDeactive(e) => Some(e.stack.base.buff_id),
            BuffRemoveSingle(e) => Some(e.remove.base.buff_id),
            BuffRemoveAll(e) => Some(e.remove.base.buff_id),
            BuffRemoveManual(e) => Some(e.base.buff_id),
            AnimatedCast(e) | GadgetInteract(e) => Some(e.skill_id),
            Emote(e) => Some(e.base.skill_id),
            BundlePickUp(e) => Some(e.base.skill_id),
            WeaponSwap(_) => Some(gw2ei_parse::skill_ids::WEAPON_SWAP as u32),
            _ => None,
        }
    }
}
