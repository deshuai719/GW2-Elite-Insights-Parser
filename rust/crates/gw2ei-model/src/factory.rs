//! 事件化工厂：把 ParseCombatList 产出的 combat 行转成类型化事件。
//!
//! 对齐 `CombatData.cs:536-704` 构造器 + `CombatEventFactory.cs` 的分发
//! （CombatData 是 partial 类）。P1 只实现新世代（build ≥ 20260501）路径；
//! 旧世代在入口显式报 `UnsupportedArcBuild`。
//!
//! 流程（对齐 C#）：
//! 1. 稳定排序（C# `combatEvents.SortByTime()`，CombatData.cs:544）。
//! 2. 第一遍：`IsEssentialMetadata` 行先行（GUID 表/单例/列表/dict 语义需要
//!    先于后续行存在，CombatData.cs:556-564）。
//! 3. 第二遍（:565-613）：cast 入桶 → buff apply/remove → direct/buff damage
//!    → 其余 state change（essential 跳过）。
//! 4. cast 配对（CreateCastEvents，CombatEventFactory.cs:735-798）。
//!
//! P1 已知偏差（记录于文档注释与最终汇报）：
//! - `From`/`To`/`Caster` 为 u64 agent 地址；玩家判定用 agent 区类型
//!   （`GW2APIController.GetSpec` 的本地子集），`CompleteAgentsAndLogData`
//!   的重定向/补缺/aware 区间语义在 P2。
//! - 坐标解包遇 NaN/Inf 丢弃（C# 位于 replay 消费端，P1 提前）——若事件
//!   流需与 C# 完全一致，该丢弃在 P2 对拍时移回消费端。

use std::collections::BTreeMap;

use gw2ei_parse::arc_builds;
use gw2ei_parse::{
    Activation, AnimationStart, AnimationStop, BuffRemove, DamageResult, EvtcCombatItem,
    EvtcRawLog, Iff, RawAgentKind, StateChange, skill_ids,
};

use crate::combat_data::{CategoryCounts, CombatData, MetaDataBucket};
use crate::error::ModelError;
use crate::state_change::dispatch_state_change;
use crate::events::*;

/// agent 区轻量查找表：地址 → agent 区原始类型。
/// C# 对应 `AgentData.GetAgent` 的 agent 区部分（不含 P2 的补缺 agent）。
#[derive(Debug, Clone, Default)]
pub struct AgentLookup {
    by_address: BTreeMap<u64, RawAgentKind>,
}

impl AgentLookup {
    pub fn new(log: &EvtcRawLog) -> Self {
        Self {
            by_address: log
                .agents
                .iter()
                .map(|a| (a.address, a.agent_kind()))
                .collect(),
        }
    }

    /// 地址在 agent 区时的类型。
    pub fn kind_of(&self, address: u64) -> Option<RawAgentKind> {
        self.by_address.get(&address).copied()
    }

    /// C# `AgentItem.IsPlayer` 的 P1 近似：地址在 agent 区且类型为 Player。
    /// 注意：补缺 agent（"UNKNOWN {addr}"，StableSpecies）不在 agent 区 →
    /// 非玩家 ✓；API 表查不到 spec 的玩家仍按 Player（对齐 GetSpec 的
    /// `default → Player` 分支）。
    pub fn is_player(&self, address: u64) -> bool {
        matches!(self.kind_of(address), Some(RawAgentKind::Player))
    }

    /// C# `AgentItem.IsUnknown` 的 P1 近似：地址 0 或不在 agent 区。
    pub fn is_unknown(&self, address: u64) -> bool {
        address == 0 || !self.by_address.contains_key(&address)
    }
}

/// `Eventize` 的中间收集状态（分桶对应 C# 构造器的局部桶）。
pub(crate) struct Collector<'a> {
    pub(crate) build: i32,
    /// 原始(归零前)日志起点(InstanceStart 的 offset 计算)。
    pub(crate) log_start: i64,
    pub(crate) agents: &'a AgentLookup,
    /// Generic breakbar id（skillData.GenericBreakbarID，用于 recovery 分流）。
    pub(crate) generic_breakbar_id: u32,
    /// 行级事件（A-F 组；metadata 进 `metadata` 桶）。
    pub(crate) events: Vec<CombatEvent>,
    pub(crate) metadata: MetaDataBucket,
    pub(crate) counts: CategoryCounts,
    pub(crate) unsupported: BTreeMap<UnsupportedEventKind, u64>,
    /// cast 待配对行：(skill_id, 排序后 items 索引)。
    pub(crate) cast_by_src: BTreeMap<u64, Vec<(u32, usize)>>,
    // per-agent/per-id 计数骨架
    pub(crate) agent_counts: BTreeMap<u64, u64>,
    pub(crate) id_counts: BTreeMap<u32, u64>,
    /// Targetable/Visibility 的连续同值去重状态（per src 尾事件）。
    pub(crate) last_targetable_by_src: BTreeMap<u64, bool>,
    pub(crate) last_visibility_by_src: BTreeMap<u64, bool>,
    /// Effect CBTS51 start 事件按 TrackingID 的配对表（事件化第二遍按
    /// 时间序访问，桶内尾部即最近的 start —— C#
    /// `EffectEventsByTrackingID` 的 LastOrDefault(x => x.Time <= Time)）。
    pub(crate) effect_starts_by_tracking: BTreeMap<u32, Vec<usize>>,
    /// Split 世代(C# EffectEventsByTrackingID 按世代分桶 ——
    /// `GroundEffectEventsByTrackingID`/`AgentEffectEventsByTrackingID`):
    /// EffectGroundCreate start 的 tracking(行 Pad)。
    pub(crate) split_ground_by_tracking: BTreeMap<u32, Vec<usize>>,
    /// EffectAgentCreate start 的 tracking(行 Pad)。
    pub(crate) split_agent_by_tracking: BTreeMap<u32, Vec<usize>>,
}

impl<'a> Collector<'a> {
    pub(crate) fn bump_unsupported(&mut self, kind: UnsupportedEventKind) {
        *self.unsupported.entry(kind).or_insert(0) += 1;
    }

}

/// 把 parse 层产出事件化。消费 `log`（内部排序原地进行，避免 20MB 级拷贝）。
pub fn build_combat_data(log: EvtcRawLog) -> Result<CombatData, ModelError> {
    let (build, _revision) = log.arc_version;
    // 新世代门控（CombatItem.cs:214-431 的分支点）。
    if build < arc_builds::BUFF_APPLIES_AND_REMOVES_AS_STATE_CHANGES {
        return Err(ModelError::UnsupportedArcBuild { build });
    }

    let agents = AgentLookup::new(&log);
    let (dodge_id, generic_breakbar_id) = skill_ids::dodge_and_breakbar_for_build(build);
    // 归零前的原始日志起点(C# EvtcLogOffset;InstanceStart 的 offset 计算用)。
    let log_start_offset = log.log_start_offset;

    let mut items = log.combat_events;
    // C# `combatEvents.SortByTime()`（稳定排序；CombatData.cs:544）。
    items.sort_by_key(|e| e.time);

    let mut collector = Collector {
        build,
        log_start: log_start_offset,
        agents: &agents,
        generic_breakbar_id,
        events: Vec::with_capacity(items.len()),
        metadata: MetaDataBucket::default(),
        counts: CategoryCounts::default(),
        unsupported: BTreeMap::new(),
        cast_by_src: BTreeMap::new(),
        agent_counts: BTreeMap::new(),
        id_counts: BTreeMap::new(),
        last_targetable_by_src: BTreeMap::new(),
        last_visibility_by_src: BTreeMap::new(),
        effect_starts_by_tracking: BTreeMap::new(),
        split_ground_by_tracking: BTreeMap::new(),
        split_agent_by_tracking: BTreeMap::new(),
    };

    // 第一遍：essential metadata 先行（CombatData.cs:556-564）。
    for item in &items {
        if item.is_essential_metadata() {
            dispatch_state_change(&mut collector, item);
        }
    }

    // 第二遍（CombatData.cs:565-613）。
    for (index, item) in items.iter().enumerate() {
        // IsCastEvent（新世代：AnimationStart/Stop 行，CombatItem.cs:407-431）
        if item.state_change == StateChange::AnimationStart
            || item.state_change == StateChange::AnimationStop
        {
            collector
                .cast_by_src
                .entry(item.src_agent)
                .or_default()
                .push((item.skill_id, index));
            continue;
        }
        // buff apply / remove（新世代独立 state change 行）
        if matches!(
            item.state_change,
            StateChange::BuffApply
                | StateChange::BuffChange
                | StateChange::BuffInitial
                | StateChange::BuffRemoveAll
                | StateChange::BuffRemoveSingle
        ) {
            if matches!(
                item.state_change,
                StateChange::BuffApply | StateChange::BuffChange | StateChange::BuffInitial
            ) {
                add_buff_apply(&mut collector, item);
            } else {
                add_buff_remove(&mut collector, item);
            }
            continue;
        }
        // 伤害行（Combat + IsBuff 分 direct/buff damage）
        if item.state_change == StateChange::Combat {
            if item.buff == 0 {
                add_direct_damage(&mut collector, item);
            } else {
                add_buff_damage(&mut collector, item);
            }
            continue;
        }
        // 其余 state change（essential 已第一遍处理）
        if item.is_essential_metadata() {
            continue;
        }
        if item.is_extension() {
            // C# 交给 handler.InsertEIExtensionEvent（extension 行语义 P3）。
            collector.bump_unsupported(UnsupportedEventKind::Extension);
            continue;
        }
        dispatch_state_change(&mut collector, item);
    }

    // cast 配对（CombatEventFactory.cs:735-798）。log_start：P1 时间轴起点为 0
    //（C# logData.EvtcLogStart 的 P2 offset 语义，见 create_cast_events 文档）。
    create_cast_events(&mut collector, &items, dodge_id, log.log_end_time, 0);

    // events 全局按时间排序（C# 各桶内各自有序；P1 提供单一时间序便于
    // 调试与测试。GUID 行在 metadata 桶，events 全部带时间）。
    collector.events.sort_by_key(|e| e.time().unwrap_or(i64::MIN));

    // per-agent / per-id 计数骨架：事件语义化完成后统一汇总（P2 在
    // CompleteAgentsAndLogData 后换成携带索引的字典）。
    for evt in &collector.events {
        let (a, b) = evt.participants();
        if a != 0 {
            *collector.agent_counts.entry(a).or_insert(0) += 1;
        }
        if b != 0 {
            *collector.agent_counts.entry(b).or_insert(0) += 1;
        }
        if let Some(id) = evt.related_id() {
            *collector.id_counts.entry(id).or_insert(0) += 1;
        }
    }

    Ok(CombatData {
        events: collector.events,
        metadata: collector.metadata,
        counts: collector.counts,
        unsupported: collector.unsupported,
        per_agent_counts: collector.agent_counts,
        per_id_counts: collector.id_counts,
        arc_version: log.arc_version,
        log_start_offset: log.log_start_offset,
        log_end_time: log.log_end_time,
    })
}

// ===== 事件构造小工具 =====

fn skill_fields(item: &EvtcCombatItem) -> SkillEventFields {
    SkillEventFields {
        time: item.time,
        from: item.src_agent,
        to: item.dst_agent,
        skill_id: item.skill_id,
        iff: item.iff,
        is_over_ninety: item.is_ninety > 0,
        against_under_fifty: item.is_fifty > 0,
        is_moving: item.is_moving & 1 > 0,
        against_moving: item.is_moving & 2 > 0,
        is_flanking: item.is_flanking > 0,
        against_downed: false,
    }
}

/// NonDamage 行公共标记：AgainstDowned=offcycle==1（NonDamageEvent.cs:10-13）。
fn skill_fields_non_damage(item: &EvtcCombatItem) -> SkillEventFields {
    let mut s = skill_fields(item);
    s.against_downed = item.is_offcycle == 1;
    s
}

/// round 语义对齐 C# `Math.Round(x, digits)`（四舍六入五成双）。
fn round_csharp(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    let scaled = value * factor;
    // Math.Round 的默认模式是 banker's rounding（ToEven）。
    (scaled.round_ties_even()) / factor
}

// ===== 伤害分派（CombatEventFactory.cs:800-903）=====

fn add_direct_damage(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    let result = DamageResult::from_byte(item.result);
    match result {
        // 非伤害结果 → 破蔑/控场/打断族
        DamageResult::BreakbarDamage
        | DamageResult::CrowdControl
        | DamageResult::Interrupt
        | DamageResult::KillingBlow
        | DamageResult::Downed => add_non_damage_damage(c, item, result),
        // 直伤子集（CombatEventFactory.cs:846-854）
        DamageResult::DirectNormal
        | DamageResult::DirectCrit
        | DamageResult::DirectGlance
        | DamageResult::DirectBlock
        | DamageResult::DirectEvade
        | DamageResult::DirectOrBuffAbsorb
        | DamageResult::DirectBlind
        | DamageResult::DirectOrBuffInvert => {
            let mut skill = skill_fields(item);
            skill.against_downed = item.is_offcycle == 1;
            let mut hp = HealthDamageEventFields {
                skill,
                ..Default::default()
            };
            hp.health_damage = item.value;
            hp.shield_damage = if item.is_shields > 0 {
                item.overstack_value as i32
            } else {
                0
            };
            hp.has_crit = result == DamageResult::DirectCrit;
            hp.has_glanced = result == DamageResult::DirectGlance;
            hp.is_absorbed = matches!(
                result,
                DamageResult::DirectOrBuffAbsorb | DamageResult::DirectOrBuffInvert
            );
            hp.is_blind = result == DamageResult::DirectBlind;
            hp.is_blocked = result == DamageResult::DirectBlock;
            hp.is_evaded = result == DamageResult::DirectEvade;
            hp.has_hit =
                result == DamageResult::DirectNormal || hp.has_glanced || hp.has_crit;
            c.events.push(CombatEvent::DirectHealthDamage(hp));
            c.counts.direct_health_damage += 1;
        }
        // Activation/Unknown/Buff 系结果落在直伤行 → C# default 丢弃。
        _ => c.counts.dropped_by_cs += 1,
    }
}

fn add_buff_damage(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    let result = DamageResult::from_byte(item.result);
    match result {
        // 症状行也能承载破蔑/控场/打断结果 → NonDamage 族
        DamageResult::BreakbarDamage
        | DamageResult::CrowdControl
        | DamageResult::Interrupt
        | DamageResult::KillingBlow
        | DamageResult::Downed => add_non_damage_damage(c, item, result),
        // 症状/间接伤害子集（CombatEventFactory.cs:878-885）
        DamageResult::BuffNotCycle
        | DamageResult::BuffCycle
        | DamageResult::BuffNotCycle_DamageToSourceOnHit
        | DamageResult::BuffNotCycle_DamageToTargetOnHit
        | DamageResult::BuffNotCycle_DamageToTargetOnStackRemove
        | DamageResult::DirectOrBuffAbsorb
        | DamageResult::DirectOrBuffInvert => {
            let mut hp = HealthDamageEventFields {
                skill: skill_fields_non_damage(item),
                ..Default::default()
            };
            hp.health_damage = item.buff_dmg;
            hp.is_life_leech = matches!(
                result,
                DamageResult::BuffNotCycle_DamageToTargetOnHit
                    | DamageResult::BuffNotCycle_DamageToTargetOnStackRemove
            );
            hp.is_absorbed = matches!(
                result,
                DamageResult::DirectOrBuffAbsorb | DamageResult::DirectOrBuffInvert
            );
            hp.shield_damage = if item.is_shields > 0 {
                if item.overstack_value > 0 {
                    item.overstack_value as i32
                } else {
                    hp.health_damage
                }
            } else {
                0
            };
            hp.has_hit = matches!(
                result,
                DamageResult::BuffCycle
                    | DamageResult::BuffNotCycle
                    | DamageResult::BuffNotCycle_DamageToSourceOnHit
            ) || hp.is_life_leech;
            c.events.push(CombatEvent::NonDirectHealthDamage(hp));
            c.counts.non_direct_health_damage += 1;
        }
        _ => c.counts.dropped_by_cs += 1,
    }
}

fn add_non_damage_damage(
    c: &mut Collector<'_>,
    item: &EvtcCombatItem,
    result: DamageResult,
) {
    match result {
        DamageResult::BreakbarDamage => {
            let value = round_csharp(item.value as f64 / 10.0, 1);
            // BreakbarChangeEvent 中间体分流（CombatEventFactory.cs:808-818）：
            // 自然恢复 = GenericBreakbarID 且来源 unknown。
            if item.skill_id == c.generic_breakbar_id && c.agents.is_unknown(item.src_agent) {
                c.events.push(CombatEvent::BreakbarRecovery(BreakbarEventFields {
                    skill: skill_fields_non_damage(item),
                    value,
                }));
                c.counts.breakbar_recovery += 1;
            } else {
                c.events.push(CombatEvent::BreakbarDamage(BreakbarEventFields {
                    skill: skill_fields_non_damage(item),
                    value,
                }));
                c.counts.breakbar_damage += 1;
            }
        }
        DamageResult::CrowdControl => {
            // Duration=Value、DefianceCalculation=Value+Overstack（int cast）。
            c.events.push(CombatEvent::CrowdControl(CrowdControlEventFields {
                skill: skill_fields_non_damage(item),
                duration: item.value,
                defiance_calculation: item.value.wrapping_add(item.overstack_value as i32),
            }));
            c.counts.crowd_control += 1;
        }
        // NoDamage 行占位（result ∈ Interrupt/KillingBlow/Downed）。
        DamageResult::Interrupt | DamageResult::KillingBlow | DamageResult::Downed => {
            // 注意：NoDamageHealthDamageEvent 继承 HealthDamageEvent（非
            // NonDamageEvent），构造器不设 AgainstDowned → 恒 false
            //（NoDamageHealthDamageEvent.cs:8-15）；此处不能用
            // skill_fields_non_damage。
            let mut hp = HealthDamageEventFields {
                skill: skill_fields(item),
                ..Default::default()
            };
            hp.has_downed = result == DamageResult::Downed;
            hp.has_killed = result == DamageResult::KillingBlow;
            hp.has_interrupted = result == DamageResult::Interrupt;
            hp.is_not_a_damage_event = true;
            c.events.push(CombatEvent::NoDamageHealthDamage(hp));
            c.counts.no_damage_health_damage += 1;
        }
        _ => {}
    }
}

// ===== Buff apply / remove 分派（CombatEventFactory.cs:652-718）=====

fn buff_fields(item: &EvtcCombatItem, by: u64, to: u64) -> BuffEventFields {
    BuffEventFields {
        time: item.time,
        buff_id: item.skill_id,
        by,
        to,
        iff: item.iff,
    }
}

fn add_buff_apply(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    let base = BuffApplyBaseFields {
        base: buff_fields(item, item.src_agent, item.dst_agent),
        buff_instance: item.pad,
    };
    if item.state_change == StateChange::BuffChange {
        // BuffExtensionEvent（BuffChange=70 行，AddBuffApplyEvent 新世代分支）
        let fields = BuffExtensionEventFields {
            apply: base,
            new_duration: item.overstack_value,
            extended_duration: i64::from(item.value.max(0)),
        };
        c.events.push(CombatEvent::BuffExtension(fields));
        c.counts.buff_extension += 1;
    } else {
        let initial = item.state_change == StateChange::BuffInitial;
        // OriginalAppliedDuration：Initial 且 build ≥ 20231107 时取 BuffDmg
        //（BuffApplyEvent.cs:25-34）。
        let applied_duration = item.value;
        let original_applied_duration = if initial
            && c.build >= arc_builds::BUFF_EXTENSION_OVERSTACK_VALUE_CHANGED
        {
            item.buff_dmg
        } else {
            applied_duration
        };
        c.events.push(CombatEvent::BuffApply(BuffApplyEventFields {
            apply: base,
            initial,
            applied_duration,
            original_applied_duration,
            added_active: item.is_shields > 0,
        }));
        c.counts.buff_apply += 1;
    }
}

fn add_buff_remove(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    // 方向修正：By=DstAgent、To=SrcAgent（AbstractBuffRemoveEvent.cs:13-15）。
    let base = BuffRemoveBaseFields {
        base: buff_fields(item, item.dst_agent, item.src_agent),
        removed_duration: item.value,
    };
    if item.state_change == StateChange::BuffRemoveAll {
        c.events.push(CombatEvent::BuffRemoveAll(BuffRemoveAllEventFields {
            remove: base,
            removed_stacks: item.result,
            last_removed_duration: item.buff_dmg,
        }));
        c.counts.buff_remove_all += 1;
    } else {
        match item.buff_remove {
            // BuffRemoveSingle 行携带 IsBuffRemove 字节区分 Single/Manual
            //（CombatEventFactory.cs:692-700）。
            BuffRemove::Single => {
                // C# OverstackOrNaturalEnd（BuffRemoveSingleEvent.cs:11）：
                // `IFF==Unknown && CreditedBy.IsUnknown && !_byShouldntBeUnknown`，
                // 其中 `_byShouldntBeUnknown = DstAgent != 0`、`CreditedBy` =
                // By = GetAgent(DstAgent)。代入后：
                // - DstAgent==0 → _byShouldntBeUnknown=false、GetAgent 恒返
                //   _unknownAgent(IsUnknown) → 条件 ≡ IFF==Unknown；
                // - DstAgent!=0 → _byShouldntBeUnknown=true → 恒 false。
                // 因此整式 ≡ `IFF==Unknown && DstAgent==0`。三个合取项里
                // `is_unknown(dst_agent)` 由 dst==0 蕴含（冗余但自文档）；
                // C# 侧 aware 窗口导致的 IsUnknown 差异因 `_byShouldntBeUnknown`
                // 短路而不影响结果。
                let overstack_or_natural_end = item.iff == Iff::Unknown
                    && c.agents.is_unknown(item.dst_agent)
                    && item.dst_agent == 0;
                c.events.push(CombatEvent::BuffRemoveSingle(
                    BuffRemoveSingleEventFields {
                        remove: base,
                        buff_instance: item.pad,
                        overstack_or_natural_end,
                    },
                ));
                c.counts.buff_remove_single += 1;
            }
            BuffRemove::Manual => {
                c.events.push(CombatEvent::BuffRemoveManual(base));
                c.counts.buff_remove_manual += 1;
            }
            // None/All/Unknown：C# switch 无分支 → 无输出。
            _ => c.counts.dropped_by_cs += 1,
        }
    }
}

// ===== Cast 配对（CombatEventFactory.cs:735-798）=====

/// 逐 (src, skill) 组配对 start/end，产出 AnimatedCast 族事件。
/// `log_start`：P1 中为 0（ParseCombatList 时间归零后的时间轴起点；
/// C# 用 `logData.EvtcLogStart`，含 P2 `OffsetEvtcData` 的 encounter 偏移，
/// P2 校准）。
fn create_cast_events(
    c: &mut Collector<'_>,
    items: &[EvtcCombatItem],
    dodge_id: u32,
    log_end: i64,
    log_start: i64,
) {
    let mut cast_events: Vec<CombatEvent> = Vec::new();
    for group in c.cast_by_src.values() {
        // 每 src 的桶内再按 skill 分组（BTreeMap 保序 + 组内保持流序；
        // C# GroupBy(x => (long)SkillID) 在稳定排序输入下同语义）。
        let mut by_skill: BTreeMap<u32, Vec<&EvtcCombatItem>> = BTreeMap::new();
        for &(skill_id, idx) in group {
            by_skill.entry(skill_id).or_default().push(&items[idx]);
        }
        let mut by_src_result: Vec<CombatEvent> = Vec::new();
        for (skill_id, sequence) in by_skill {
            let mut start: Option<&EvtcCombatItem> = None;
            let mut paired: Vec<CombatEvent> = Vec::new();
            for item in sequence {
                if item.state_change == StateChange::AnimationStart {
                    // 前一个 start 无 end → 孤儿 start flush（:752-757）。
                    if let Some(prev_start) = start.take() {
                        paired.push(create_cast_event(
                            Some(prev_start), None, dodge_id, log_end, skill_id,
                        ));
                    }
                    start = Some(item);
                } else {
                    // AnimationStop 行
                    if let Some(prev_start) = start.take() {
                        paired.push(create_cast_event(
                            Some(prev_start), Some(item), dodge_id, log_end, skill_id,
                        ));
                    } else {
                        // 孤儿 end：仅当起算后时间 < logStart 才保留
                        //（:767-775）。
                        let evt = create_cast_event(
                            None, Some(item), dodge_id, log_end, skill_id,
                        );
                        if evt.time().expect("cast event has time") < log_start {
                            paired.push(evt);
                        }
                    }
                }
            }
            // 尾部孤儿 start（:779-783）。
            if let Some(prev_start) = start {
                paired.push(create_cast_event(
                    Some(prev_start), None, dodge_id, log_end, skill_id,
                ));
            }
            // C# RemoveAll：玩家 cast 且 ActualDuration<=1ms 丢弃（:784）。
            paired.retain(|e| {
                let (caster, actual) = cast_identity(e);
                !(c.agents.is_player(caster) && actual <= 1)
            });
            by_src_result.extend(paired);
        }
        // per-src 时间排序 + 相邻 CutAt 防重叠（:787-793，
        // ServerDelayConstant=150，ParserHelper.cs:30）。
        by_src_result.sort_by_key(CombatEvent::time_key_unchecked);
        for i in 0..by_src_result.len().saturating_sub(1) {
            let max_end = by_src_result[i + 1].time_key_unchecked() + 150;
            cut_at(&mut by_src_result[i], max_end);
        }
        cast_events.extend(by_src_result);
    }
    // 全局按时间排序（:796）。
    cast_events.sort_by_key(CombatEvent::time_key_unchecked);
    for evt in cast_events {
        match evt {
            CombatEvent::AnimatedCast(_) => c.counts.animated_cast += 1,
            CombatEvent::Emote(_) => c.counts.emote += 1,
            CombatEvent::GadgetInteract(_) => c.counts.gadget_interact += 1,
            CombatEvent::BundlePickUp(_) => c.counts.bundle_pick_up += 1,
            _ => unreachable!("cast pairing only emits animated cast family"),
        }
        c.events.push(evt);
    }
}

fn cast_identity(evt: &CombatEvent) -> (u64, i32) {
    match evt {
        CombatEvent::AnimatedCast(f) => (f.caster, f.actual_duration),
        CombatEvent::Emote(f) => (f.base.caster, f.base.actual_duration),
        CombatEvent::GadgetInteract(f) => (f.caster, f.actual_duration),
        CombatEvent::BundlePickUp(f) => (f.base.caster, f.base.actual_duration),
        _ => (0, i32::MIN),
    }
}

impl CombatEvent {
    fn time_key_unchecked(&self) -> i64 {
        self.time().expect("cast events always carry time")
    }
}

/// `CutAt`：EndTime > maxEnd 且 status Unknown 时截断 ActualDuration
///（AnimatedCastEvent.cs:141-145）。
fn cut_at(evt: &mut CombatEvent, max_end: i64) {
    let (time, actual, status) = match evt {
        CombatEvent::AnimatedCast(f) => (f.time, f.actual_duration, f.status),
        CombatEvent::Emote(f) => (f.base.time, f.base.actual_duration, f.base.status),
        CombatEvent::GadgetInteract(f) => (f.time, f.actual_duration, f.status),
        CombatEvent::BundlePickUp(f) => (f.base.time, f.base.actual_duration, f.base.status),
        _ => return,
    };
    if status == AnimationStatus::Unknown && time + i64::from(actual) > max_end {
        let new_actual = (max_end - time) as i32;
        match evt {
            CombatEvent::AnimatedCast(f) => f.actual_duration = new_actual,
            CombatEvent::Emote(f) => f.base.actual_duration = new_actual,
            CombatEvent::GadgetInteract(f) => f.actual_duration = new_actual,
            CombatEvent::BundlePickUp(f) => f.base.actual_duration = new_actual,
            _ => {}
        }
    }
}

/// 创建一条 AnimatedCast 族事件并分派（CombatEventFactory.cs:720-733）。
fn create_cast_event(
    start: Option<&EvtcCombatItem>,
    end: Option<&EvtcCombatItem>,
    dodge_id: u32,
    log_end: i64,
    skill_id: u32,
) -> CombatEvent {
    let fields = build_cast_fields(start, end, dodge_id, log_end, skill_id);
    // 按 SkillID 特殊值分派（build ≥ 20260318 恒成立，P1 世代全命中）。
    match skill_id {
        id if id == skill_ids::ARC_GENERIC_EMOTE => {
            let emote_id = start
                .map(|s| {
                    if s.state_change == StateChange::AnimationStart {
                        s.overstack_value as u64
                    } else {
                        // 旧世代（P1 不达）的 Pad 承载。
                        s.pad as u64
                    }
                })
                .unwrap_or(0);
            CombatEvent::Emote(EmoteEventFields {
                base: fields,
                emote_id,
            })
        }
        id if id == skill_ids::ARC_GENERIC_GADGET_INTERACT => {
            // GadgetInteract 构造分支的中断状态修正（GadgetInteractEvent.cs:16-21）。
            let mut fields = fields;
            apply_gadget_interact_fix(&mut fields);
            CombatEvent::GadgetInteract(fields)
        }
        id if id == skill_ids::ARC_GENERIC_PICK_UP => {
            let bundle_id = start.map(|s| s.overstack_value as u64).unwrap_or(0);
            // Caster 断开（BundlePickUpEvent.cs:12-13）。
            let mut fields = fields;
            fields.caster = 0;
            CombatEvent::BundlePickUp(BundlePickUpEventFields {
                base: fields,
                bundle_id,
            })
        }
        _ => CombatEvent::AnimatedCast(fields),
    }
}

/// `AnimatedCastEvent` 构造（AnimatedCastEvent.cs:41-133）。
fn build_cast_fields(
    start: Option<&EvtcCombatItem>,
    end: Option<&EvtcCombatItem>,
    dodge_id: u32,
    log_end: i64,
    skill_id: u32,
) -> AnimatedCastEventFields {
    let (start_item, end_item) = (start, end);
    let base_item = start_item.or(end_item).expect("either start or end item");
    let is_dodge = skill_id == dodge_id;
    let is_resurrect = skill_id as i64 == skill_ids::RESURRECT;

    let mut f = AnimatedCastEventFields {
        time: base_item.time,
        skill_id,
        caster: base_item.src_agent,
        effect_target: 0,
        anim_start: AnimationStart::None,
        anim_stop: AnimationStop::None,
        status: AnimationStatus::Unknown,
        saved_duration: 0,
        expected_duration: 0,
        actual_duration: 0,
        acceleration: 0.0,
    };
    // nonScaledToScaledRatio（SetAcceleration 的换算根；GadgetInteract 还原用）。
    let mut non_scaled_ratio = 1.0f64;

    // ===== start 分支（AnimatedCastEvent.cs:49-71）=====
    if let Some(start_item) = start_item {
        // ExpectedDuration = BuffDmg>0 ? BuffDmg : Value（:51-52）。
        f.expected_duration = if start_item.buff_dmg > 0 {
            start_item.buff_dmg
        } else {
            start_item.value
        };
        if start_item.state_change == StateChange::AnimationStart {
            // 新世代：EffectTarget = start 行 DstAgent（:53-59）。
            if start_item.dst_agent != 0 {
                f.effect_target = start_item.dst_agent;
            }
        } else if start_item.activation == Activation::Quickness {
            // 旧世代（P1 不达）：Quickness 加速标记。
            f.acceleration = 1.0;
        }
        f.anim_start = AnimationStart::from_byte(start_item.result);
    }

    // ===== end 分支（AnimatedCastEvent.cs:73-124）=====
    if let Some(end_item) = end_item {
        // ActualDuration = end.Value（:92，动画实际时长）；BuffDmg 是
        // scaled 刻度（:93，0 表示无刻度数据）。新世代 AnimationStop 行的
        // Value 仍承载实际时长（20260530 样例实证），不可省略。
        f.actual_duration = end_item.value;
        let mut scaled_actual_duration = end_item.buff_dmg;
        if let Some(start_item) = start_item {
            // sanity check：|Actual - (end.Time - start.Time)| > 150ms 用差值
            //（:96-100，ServerDelayConstant）。
            let expected_actual = end_item.time.wrapping_sub(start_item.time);
            if (i64::from(f.actual_duration) - expected_actual).abs() > 150 {
                f.actual_duration = expected_actual as i32;
                scaled_actual_duration = 0;
            }
            if is_dodge {
                // dodge start 行的 expected 恒 0（:101-104）。
                f.expected_duration = f.actual_duration;
                scaled_actual_duration = 0;
            }
        } else {
            // 孤儿 end：Expected=Actual、dodge 时 scaled 置 0、Time 前移
            //（:97-104，Time -= Actual 把事件挪到起算点）。
            f.expected_duration = f.actual_duration;
            if is_dodge {
                scaled_actual_duration = 0;
            }
            f.time = f.time.wrapping_sub(i64::from(f.actual_duration));
        }
        f.anim_stop = AnimationStop::from_byte(end_item.result);
        set_acceleration(
            &mut f,
            end_item,
            scaled_actual_duration,
            &mut non_scaled_ratio,
            is_resurrect,
        );
        // GadgetInteract 的 Expected 刻度还原：C# 里
        // ExpectedDuration /= AcceleratedToNonAcceleratedRatio（= ×
        // nonScaledRatio），随后 Reduced 分支的 SavedDuration 再乘回；
        // 净效应只是换刻度，Saved 值不变，这里仅对 Reduced 重算以对齐。
        if f.anim_start == AnimationStart::GadgetInteract && non_scaled_ratio != 1.0 {
            f.expected_duration = (f.expected_duration as f64 * non_scaled_ratio) as i32;
            if f.status == AnimationStatus::Reduced {
                // C# `(int)Math.Round(...)`（ToEven，GadgetInteractEvent.cs:25）。
                let scaled_expected =
                    round_csharp(f.expected_duration as f64 / non_scaled_ratio, 0) as i32;
                f.saved_duration = (scaled_expected - f.actual_duration).max(0);
            }
        }
    } else {
        // 孤儿 start：dodge → Expected=750；Actual=Expected；CutAt(logEnd)
        //（:66-70）。
        if is_dodge {
            f.expected_duration = 750;
        }
        f.actual_duration = f.expected_duration;
        if f.time + i64::from(f.actual_duration) > log_end
            && f.status == AnimationStatus::Unknown
        {
            f.actual_duration = (log_end - f.time) as i32;
        }
    }
    f
}

/// `SetAcceleration`（AnimatedCastEvent.cs:14-36）：按 end 行刻度与 Activation
/// 计算 acceleration / status / saved。
#[allow(clippy::too_many_arguments)]
fn set_acceleration(
    f: &mut AnimatedCastEventFields,
    end_item: &EvtcCombatItem,
    scaled_actual_duration: i32,
    non_scaled_ratio: &mut f64,
    is_resurrect: bool,
) {
    if scaled_actual_duration > 0 {
        *non_scaled_ratio = scaled_actual_duration as f64 / f.actual_duration as f64;
        if *non_scaled_ratio > 1.0 {
            // faster
            f.acceleration = (*non_scaled_ratio - 1.0) / 0.5;
        } else {
            f.acceleration = -(1.0 - *non_scaled_ratio) / 0.6;
        }
        f.acceleration = f.acceleration.clamp(-1.0, 1.0);
    }
    if !is_resurrect {
        match end_item.activation {
            Activation::Cancel => {
                f.status = AnimationStatus::Interrupted;
                f.saved_duration = -f.actual_duration;
            }
            Activation::Reset => {
                f.status = AnimationStatus::Full;
            }
            Activation::Minimum | Activation::NoData => {
                // C# `(int)Math.Round(Expected / ratio)` —— ToEven（:44）。
                let scaled_expected =
                    round_csharp(f.expected_duration as f64 / *non_scaled_ratio, 0) as i32;
                f.saved_duration = (scaled_expected - f.actual_duration).max(0);
                f.status = AnimationStatus::Reduced;
            }
            _ => {}
        }
    }
    f.acceleration = round_csharp(f.acceleration, 3);
}

/// GadgetInteract 的中断状态修正（GadgetInteractEvent.cs:16-21）：正常流程
/// 下 AnimStop 非 GadgetViaReset/Ended 且状态非 Interrupted → 判定为中断。
/// Expected 刻度换算在 `build_cast_fields` end 分支内完成。
fn apply_gadget_interact_fix(f: &mut AnimatedCastEventFields) {
    if f.anim_start != AnimationStart::GadgetInteract {
        return;
    }
    if f.anim_stop != AnimationStop::GadgetViaReset
        && f.anim_stop != AnimationStop::Ended
        && f.status != AnimationStatus::Interrupted
    {
        f.status = AnimationStatus::Interrupted;
        f.saved_duration = -f.actual_duration;
    }
}
