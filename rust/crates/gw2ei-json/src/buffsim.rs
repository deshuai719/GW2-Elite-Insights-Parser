#![allow(clippy::doc_lazy_continuation)]
//! per-actor BuffSimulator 装配与 buff 统计查询（P3a）。
//!
//! 流水线（严格按 C# 顺序，逐段对照）：
//! 1. 行分桶（CombatData ctor 的 `_buffDataByDst`）：buff 事件按 To 解析
//!    agent 分桶（每 friend root 一桶；解析语义同 rows.rs `resolve`）。
//! 2. Extension overstack 修正（CombatData.cs:467-533 `OffsetBuffExtensionEvents`
//!    + BuffExtensionEvent.OffsetNewDuration:35-108）：按 (BuffInstance,
//!    BuffID) 序列折算真实增量；修正后 ExtendedDuration<1 的从桶剔除。
//! 3. TryFindSrc（BuffsContainer.cs:261-263 + BuffSourceFinders 现代链
//!    20260113）：extension 的 By unknown 时按 buff 源推断（非 boon：seed
//!    apply；boon：Ranger/Soulbeast trait + 技能时长表）。
//! 4. BuffDictionary 清洗（BuffDictionary.cs:21-158）：合规过滤 + 去重 +
//!    AddRegen + despawn/spawn/尾部注入 RemoveAll（ServerDelay=10ms）。
//! 5. 稳定排序（SortByTime 稳定）+ 逐 buff 仿真（gw2ei-semantics::sim）。
//!
//! 查询面：每 friend 的 whole-log 分布（`Distrib`）；JSON 各 buff 块在此
//! 之上组装（见 build.rs）。
//!
//! 有意偏差（design.md 登记）：englobed/split 段分摊不做（样例无 split）；
//! TryFindSrc 只移植现代链（20240319+，样例 build 20260507 命中 20260113：
//! imperial impact / imbued melodies 恒失效、ResoundingTimbre=2000）。

use std::collections::{BTreeMap, BTreeSet};

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, AgentTable, NO_AGENT};
use gw2ei_parse::BuffStackType;
use gw2ei_semantics::sim::{self, SimCmd, SimResult, Simulator};
use gw2ei_semantics::stats::{self, Distrib};

use crate::content::BuffDef;
use crate::ctx::Ctx;

/// SkillIDs.Regeneration（StackActive 事件仅它进 NoID 仿真）。
pub const REGEN_BUFF_ID: i64 = 718;
/// SkillIDs.NumberOfBoons / NumberOfConditions（伪图键）。
pub const NUMBER_OF_BOONS: i64 = -3;
pub const NUMBER_OF_CONDITIONS: i64 = -4;
/// ParserHelper.ServerDelayConstant（注入 RemoveAll 的偏移）。
pub const SERVER_DELAY: i64 = 10;

// ===== 行结构 =====

/// 分桶行。by 为已解析的 credited agent（Ext 行待 TryFindSrc 改写）。
#[derive(Debug, Clone)]
pub struct Row {
    pub ev: usize,
    pub time: i64,
    pub by: AgentId,
    pub buff_id: i64,
    pub kind: RowKind,
}

#[derive(Debug, Clone)]
pub enum RowKind {
    Apply {
        instance: u32,
        applied: i64,
        original: i64,
        initial: bool,
        added_active: bool,
        /// AddRegen 的 OverridenRegen*（(duration, instance)）。
        regen_override: Option<(i64, u64)>,
    },
    Ext {
        instance: u32,
        /// post-offset 值（OffsetNewDuration 原地改）。
        new_dur: i64,
        ext_dur: i64,
        /// 原始（OffsetNewDuration 的 `OriginalNewDuration`）。
        original_new: i64,
    },
    RemoveSingle {
        instance: u32,
        removed: i64,
        overstack_or_natural_end: bool,
    },
    RemoveAll,
    StackActive {
        instance: u32,
    },
    StackDeactive {
        instance: u32,
        reset_to: i64,
    },
}

/// 地址 → agent（rows.rs `resolve`：aware 优先，兜底同地址任意槽）。
pub fn resolve_addr(table: &mut AgentTable, addr: u64, time: i64) -> AgentId {
    if addr == 0 {
        return NO_AGENT;
    }
    let id = table.resolve_agent(addr, time);
    if id != NO_AGENT {
        return id;
    }
    table
        .all_ids()
        .into_iter()
        .find(|&x| table.slot(x).expect("slot must exist").address == addr)
        .unwrap_or(NO_AGENT)
}

/// `By.GetFinalMaster()` 后 root（BuffEvent.cs:12 CreditedBy；
/// BuffsContainer.TryFindSrc 的 .EnglobingAgentItem 同义）。
pub fn credited(table: &AgentTable, id: AgentId) -> AgentId {
    if id == NO_AGENT {
        return NO_AGENT;
    }
    table.englobing_root(table.final_master(id))
}

/// BuffDef.stack_type（buff-table.json 存 C# 枚举名）。
pub fn parse_stack_type(s: &str) -> Option<BuffStackType> {
    Some(match s {
        "Queue" => BuffStackType::Queue,
        "Regeneration" => BuffStackType::Regeneration,
        "Force" => BuffStackType::Force,
        "Stacking" => BuffStackType::Stacking,
        "StackingUniquePerSrc" => BuffStackType::StackingUniquePerSrc,
        "StackingConditionalLoss" => BuffStackType::StackingConditionalLoss,
        _ => return None,
    })
}

// ===== 产物 =====

/// (actor, buff) whole-log 仿真产物。
pub struct OneBuff {
    pub buff: BuffDef,
    pub result: SimResult<AgentId>,
    /// whole-log phase 归约（JSON 单 phase = 全日志）。
    pub distrib: Distrib<AgentId>,
    pub intensity: bool,
}

pub struct ActorSims {
    /// buff id（signed）→ 产物（键集 = 仿真过的 buff）。
    pub by_buff: BTreeMap<i64, OneBuff>,
    /// whole-log active duration（dead+dc 扣除；active 变体分母）。
    pub active_duration: i64,
}

pub struct BuffSims {
    pub per_actor: BTreeMap<AgentId, ActorSims>,
}

// ===== 全局辅助索引 =====

struct Globals {
    /// (buff_id u32, instance) → 全日志 BuffApply 元（判重 / seed 查询）。
    applies_by_inst: BTreeMap<(u32, u32), Vec<ApplyMeta>>,
    /// Boon 分类 id（TryFindSrc 分支）。
    boon_ids: BTreeSet<i64>,
    /// 玩家 AnimatedCast：(skill_id, caster, start, end)。
    casts: Vec<(i64, AgentId, i64, i64)>,
    /// BuffInfo duration cap（finder 的 DurationCap==0 分支差异）。
    dur_cap: BTreeMap<i64, u32>,
    /// BuffInfo stacking type（offset 的 forceActive 参数）。
    stack_type_info: BTreeMap<i64, BuffStackType>,
}

#[derive(Debug, Clone, Copy)]
struct ApplyMeta {
    time: i64,
    by: AgentId,
    initial: bool,
}

fn build_globals(ctx: &Ctx<'_>) -> Globals {
    let log = ctx.log;
    let mut resolver = log.agents.clone();
    let mut g = Globals {
        applies_by_inst: BTreeMap::new(),
        boon_ids: BTreeSet::new(),
        casts: Vec::new(),
        dur_cap: BTreeMap::new(),
        stack_type_info: BTreeMap::new(),
    };
    g.boon_ids.extend(ctx.registry.boon_ids());
    for (&id, evt) in &log.metadata.buff_info_by_id {
        if let Some(info) = evt.info.as_ref() {
            let sid = crate::signed_id(id);
            g.dur_cap.insert(sid, info.duration_cap);
            g.stack_type_info.insert(sid, info.stacking_type);
        }
    }
    for evt in &log.events {
        match evt {
            CombatEvent::BuffApply(f) => {
                if f.apply.buff_instance != 0 {
                    let by = resolve_addr(&mut resolver, f.apply.base.by, f.apply.base.time);
                    g.applies_by_inst
                        .entry((f.apply.base.buff_id, f.apply.buff_instance))
                        .or_default()
                        .push(ApplyMeta {
                            time: f.apply.base.time,
                            by,
                            initial: f.initial,
                        });
                }
            }
            CombatEvent::AnimatedCast(f) => {
                let caster = resolve_addr(&mut resolver, f.caster, f.time);
                if caster != NO_AGENT {
                    g.casts.push((
                        crate::signed_id(f.skill_id),
                        caster,
                        f.time,
                        f.time + i64::from(f.actual_duration),
                    ));
                }
            }
            _ => {}
        }
    }
    g
}

// ===== 主入口 =====

pub fn build_buff_sims(ctx: &Ctx<'_>) -> BuffSims {
    let g = build_globals(ctx);
    let mut store = BuffSims {
        per_actor: BTreeMap::new(),
    };
    let friends: Vec<AgentId> = ctx
        .log
        .players
        .iter()
        .map(|p| p.agent)
        .chain(ctx.log.friendly_non_squad.iter().map(|n| n.agent))
        .collect();
    for &actor in &friends {
        let sims = build_actor(ctx, &g, actor);
        store.per_actor.insert(actor, sims);
    }
    store
}

#[allow(clippy::too_many_lines)]
fn build_actor(ctx: &Ctx<'_>, g: &Globals, actor: AgentId) -> ActorSims {
    let log = ctx.log;
    let mut rows: Vec<Row> = Vec::new();
    let mut resolver = log.agents.clone();
    let mut despawn_times: Vec<i64> = Vec::new();
    let mut spawn_times: Vec<i64> = Vec::new();
    for (i, evt) in log.events.iter().enumerate() {
        match evt {
            CombatEvent::BuffApply(f) => {
                let to = resolve_addr(&mut resolver, f.apply.base.to, f.apply.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.apply.base.time,
                    by: credited(&log.agents, resolve_addr(&mut resolver, f.apply.base.by, f.apply.base.time)),
                    buff_id: crate::signed_id(f.apply.base.buff_id),
                    kind: RowKind::Apply {
                        instance: f.apply.buff_instance,
                        applied: i64::from(f.applied_duration),
                        original: i64::from(f.original_applied_duration),
                        initial: f.initial,
                        added_active: f.added_active,
                        regen_override: None,
                    },
                });
            }
            CombatEvent::BuffExtension(f) => {
                let to = resolve_addr(&mut resolver, f.apply.base.to, f.apply.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.apply.base.time,
                    by: credited(&log.agents, resolve_addr(&mut resolver, f.apply.base.by, f.apply.base.time)),
                    buff_id: crate::signed_id(f.apply.base.buff_id),
                    kind: RowKind::Ext {
                        instance: f.apply.buff_instance,
                        new_dur: i64::from(f.new_duration),
                        ext_dur: f.extended_duration,
                        original_new: i64::from(f.new_duration),
                    },
                });
            }
            CombatEvent::BuffRemoveSingle(f) => {
                let to = resolve_addr(&mut resolver, f.remove.base.to, f.remove.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.remove.base.time,
                    by: credited(&log.agents, resolve_addr(&mut resolver, f.remove.base.by, f.remove.base.time)),
                    buff_id: crate::signed_id(f.remove.base.buff_id),
                    kind: RowKind::RemoveSingle {
                        instance: f.buff_instance,
                        removed: i64::from(f.remove.removed_duration),
                        overstack_or_natural_end: f.overstack_or_natural_end,
                    },
                });
            }
            CombatEvent::BuffRemoveAll(f) => {
                let to = resolve_addr(&mut resolver, f.remove.base.to, f.remove.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.remove.base.time,
                    by: credited(&log.agents, resolve_addr(&mut resolver, f.remove.base.by, f.remove.base.time)),
                    buff_id: crate::signed_id(f.remove.base.buff_id),
                    kind: RowKind::RemoveAll,
                });
            }
            CombatEvent::BuffStackActive(f) => {
                let to = resolve_addr(&mut resolver, f.base.to, f.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.base.time,
                    by: NO_AGENT,
                    buff_id: crate::signed_id(f.base.buff_id),
                    kind: RowKind::StackActive {
                        instance: f.buff_instance,
                    },
                });
            }
            CombatEvent::BuffStackDeactive(f) => {
                let to = resolve_addr(&mut resolver, f.stack.base.to, f.stack.base.time);
                if to != actor {
                    continue;
                }
                rows.push(Row {
                    ev: i,
                    time: f.stack.base.time,
                    by: NO_AGENT,
                    buff_id: crate::signed_id(f.stack.base.buff_id),
                    kind: RowKind::StackDeactive {
                        instance: f.stack.buff_instance,
                        reset_to: i64::from(f.reset_to_duration),
                    },
                });
            }
            CombatEvent::Despawn(f) => {
                let src = resolve_addr(&mut resolver, f.src, f.time);
                if src == actor {
                    despawn_times.push(f.time);
                }
            }
            CombatEvent::Spawn(f) => {
                let src = resolve_addr(&mut resolver, f.src, f.time);
                if src == actor {
                    spawn_times.push(f.time);
                }
            }
            _ => {}
        }
    }
    // 1. rows 已按 ev 序（事件流 = 时间稳定序）
    offset_extensions(g, &mut rows);
    try_find_src(ctx, g, actor, &mut rows);
    // 2. 清洗 + 注入 + 排序 + 仿真
    let active_duration = {
        let b = crate::actors::ActorBuilder {
            ctx,
            actor: crate::rows::ActorRows::new(log, &ctx.rows, actor),
        };
        b.active_duration(actor, log.log_data.log_start, log.log_data.evtc_log_end)
    };
    cleanse_and_simulate(ctx, g, actor, rows, despawn_times, spawn_times, active_duration)
}

// ===== 2. Extension overstack 修正（CombatData.cs:467-533）=====

fn offset_extensions(g: &Globals, rows: &mut Vec<Row>) {
    let mut ext_by_inst: BTreeMap<u32, BTreeMap<i64, Vec<usize>>> = BTreeMap::new();
    let mut apply_by_inst: BTreeMap<u32, BTreeMap<i64, Vec<usize>>> = BTreeMap::new();
    let mut stack_by_inst: BTreeMap<u32, BTreeMap<i64, Vec<usize>>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        match r.kind {
            RowKind::Ext { instance, .. } if instance != 0 => {
                ext_by_inst.entry(instance).or_default().entry(r.buff_id).or_default().push(i);
            }
            RowKind::Apply { instance, .. } if instance != 0 => {
                apply_by_inst.entry(instance).or_default().entry(r.buff_id).or_default().push(i);
            }
            RowKind::StackActive { instance } | RowKind::StackDeactive { instance, .. }
                if instance != 0 =>
            {
                stack_by_inst.entry(instance).or_default().entry(r.buff_id).or_default().push(i);
            }
            _ => {}
        }
    }
    for (instance, per_buff) in &ext_by_inst {
        for (buff_id, ext_idxs) in per_buff {
            let Some(applies) = apply_by_inst.get(instance).and_then(|m| m.get(buff_id)) else {
                continue;
            };
            // buffDesc.StackingType == StackingConditionalLoss → forceActive
            let force_active = g
                .stack_type_info
                .get(buff_id)
                .is_some_and(|st| *st == BuffStackType::StackingConditionalLoss);
            let mut previous: Option<usize> = None;
            for &ext_idx in ext_idxs {
                let apply_idx = applies
                    .iter()
                    .rev()
                    .find(|&&a| rows[a].time <= rows[ext_idx].time)
                    .copied();
                let Some(apply_idx) = apply_idx else { continue };
                let mut sequence: Vec<usize> = vec![apply_idx];
                if let Some(stacks) = stack_by_inst.get(instance).and_then(|m| m.get(buff_id)) {
                    let (at, et) = (rows[apply_idx].time, rows[ext_idx].time);
                    sequence.extend(
                        stacks
                            .iter()
                            .copied()
                            .filter(|&s| rows[s].time >= at && rows[s].time <= et),
                    );
                }
                if let Some(p) = previous
                    && rows[p].time >= rows[apply_idx].time
                {
                    sequence.push(p);
                }
                previous = Some(ext_idx);
                sequence.sort_by_key(|&i| rows[i].time); // SortByTime 稳定
                let (n, e) = {
                    let RowKind::Ext {
                        new_dur, ext_dur, ..
                    } = rows[ext_idx].kind
                    else {
                        unreachable!("ext row")
                    };
                    offset_new_duration(
                        rows[ext_idx].time,
                        new_dur,
                        ext_dur,
                        &sequence,
                        rows,
                        force_active,
                    )
                };
                if let RowKind::Ext {
                    new_dur, ext_dur, ..
                } = &mut rows[ext_idx].kind
                {
                    *new_dur = n;
                    *ext_dur = e;
                }
            }
        }
    }
    // 修正后 <1ms 的剔除（CombatData.cs:528-531）
    rows.retain(|r| !matches!(r.kind, RowKind::Ext { ext_dur, .. } if ext_dur < 1));
}

/// BuffExtensionEvent.OffsetNewDuration（BuffExtensionEvent.cs:35-108）。
/// 样例 build ≥ 20231107（BuffExtensionOverstackValueChanged）→ 无旧世代
/// 二次 clamp。`row_of` 只读访问序列行。
#[allow(clippy::too_many_arguments)]
fn offset_new_duration(
    ext_time: i64,
    new_dur: i64,
    ext_dur: i64,
    sequence: &[usize],
    rows: &[Row],
    force_active: bool,
) -> (i64, i64) {
    let mut active_time: i64 = 0;
    let mut previous_time: i64 = i64::MIN;
    let mut original_stack_duration: i64 = 0;
    let mut added_extension: i64 = 0;
    let mut i = 0usize;
    while i < sequence.len() {
        let cur = &rows[sequence[i]];
        if i == 0 {
            let RowKind::Apply {
                applied,
                original,
                initial,
                ..
            } = cur.kind
            else {
                panic!("OffsetNewDuration first element should be buff apply");
            };
            original_stack_duration = original;
            if initial {
                active_time += original - applied;
            }
        } else {
            match cur.kind {
                RowKind::StackActive { .. } => {
                    // 前段栈未激活；SCL（forceActive）例外：视为持续激活
                    if force_active {
                        added_extension += cur.time - previous_time;
                        active_time += cur.time - previous_time;
                    }
                }
                RowKind::StackDeactive { reset_to, .. } => {
                    active_time += cur.time - previous_time;
                    original_stack_duration = reset_to;
                }
                RowKind::Ext {
                    original_new: on,
                    ..
                } => {
                    // 栈重置伪装（新时长 ≤ 原始栈时长 → activeTime 归零）
                    if on <= original_stack_duration {
                        active_time = 0;
                    } else {
                        i += 1;
                        continue; // C# continue 不更新 previousTime
                    }
                }
                _ => {
                    panic!("OffsetNewDuration elements after first should be StackActive/StackReset");
                }
            }
        }
        previous_time = cur.time;
        i += 1;
    }
    active_time += ext_time - previous_time;
    if new_dur <= original_stack_duration {
        return (new_dur, active_time);
    }
    (new_dur - active_time, ext_dur + added_extension)
}

// ===== 3. TryFindSrc（BuffSourceFinders 现代链 20260113）=====

/// BuffSourceFinder 常量。
const ESSENCE_OF_SPEED: i64 = 2000; // Soulbeast trait（20181211 起）
const RESOUNDING_TIMBRE: i64 = 2000; // Ranger trait（20260113）
const SOI: i64 = 10236; // Signet of Inspiration
const TN: i64 = 51696; // True Nature Dragon
const SAND_SQUALL: i64 = 29453;
/// BuffSimulatorStackActiveDelayConstant（finder 容差 = 15）。
const FINDER_TOLERANCE: i64 = 15;

#[allow(clippy::ptr_arg)]
fn try_find_src(ctx: &Ctx<'_>, g: &Globals, actor: AgentId, rows: &mut Vec<Row>) {
    let to_fix: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.kind {
            RowKind::Ext { ext_dur, .. } if r.by == NO_AGENT && ext_dur >= 1 => Some(i),
            _ => None,
        })
        .collect();
    for i in to_fix {
        let (time, buff_id, instance, ext_dur) = {
            let r = &rows[i];
            let RowKind::Ext { instance, ext_dur, .. } = r.kind else { unreachable!() };
            (r.time, r.buff_id, instance, ext_dur)
        };
        if let Some(f) = buff_source_finder(ctx, g, actor, time, buff_id, ext_dur, instance) {
            rows[i].by = f;
        }
    }
}

/// Ranger/Soulbeast 特判（CouldBeRangerTraits）。
#[derive(PartialEq, Clone, Copy)]
enum Certainty {
    Certain,
    Uncertain,
    NotApplicable,
}

/// 现代链 TryFindSrc（BuffSourceFinder20260113 → 20240319 → ...）：
/// - 非 boon：instance seed apply 兜底（BuffSourceFinder.cs:145-153）；
/// - boon：Ranger/Soulbeast trait（Certain → dst 自身）→ 技能时长表匹配
///   （单 cast → caster；无 → NoCastSrcFinder）→ 其余 unknown。
/// 20240319+ 无 imperial impact；20260113 起 ResoundingTimbre=2000；
/// ImbuedMelodies = int.MinValue（20191001 起）恒不命中。
#[allow(clippy::too_many_arguments)]
fn buff_source_finder(
    ctx: &Ctx<'_>,
    g: &Globals,
    dst: AgentId,
    time: i64,
    buff_id: i64,
    extension: i64,
    buff_instance: u32,
) -> Option<AgentId> {
    if !g.boon_ids.contains(&buff_id) {
        // 非 boon：seed apply
        if buff_instance > 0
            && let Some(list) = g.applies_by_inst.get(&(buff_id as u32, buff_instance))
            && let Some(seed) = list.iter().rev().find(|a| a.time <= time)
        {
            return Some(seed.by);
        }
        return None;
    }
    // ---- boon 分支 ----
    // GetSpecAtTime：样例无 spec 分裂（split_pieces=0）→ 槽位 spec 即
    // 全程 spec（与 C# 一致的有意偏差记录于模块头）。
    let (is_soulbeast, is_ranger) = if dst == NO_AGENT {
        (false, false)
    } else {
        let spec = ctx.log.agents.slot(dst).expect("dst slot");
        (
            spec.spec == gw2ei_model::Spec::Soulbeast,
            spec.base_spec == gw2ei_model::Spec::Ranger,
        )
    };
    let duration_cap_zero = g.dur_cap.get(&buff_id).is_some_and(|c| *c == 0);
    // CouldBeRangerTraits（20210511 覆写：DurationCap==0 用 abs 判定；
    // 否则 extension ≤ key+tolerance）
    let trait_hit = if duration_cap_zero {
        (is_soulbeast && (extension - ESSENCE_OF_SPEED).abs() <= FINDER_TOLERANCE)
            || (is_ranger && (extension - RESOUNDING_TIMBRE).abs() <= FINDER_TOLERANCE)
    } else {
        (is_soulbeast && extension <= ESSENCE_OF_SPEED + FINDER_TOLERANCE)
            || (is_ranger && extension <= RESOUNDING_TIMBRE + FINDER_TOLERANCE)
    };
    let ranger = if trait_hit {
        if !get_ids(g, buff_id, extension, duration_cap_zero).is_empty() {
            Certainty::Uncertain
        } else {
            // CouldBeImbuedMelodies / ImperialImpact 恒 false（现代）
            Certainty::Certain
        }
    } else {
        Certainty::NotApplicable
    };
    if ranger == Certainty::Certain {
        return Some(dst);
    }
    let ids_to_check = get_ids(g, buff_id, extension, duration_cap_zero);
    if ids_to_check.is_empty() {
        return no_cast_src_finder(ranger, dst);
    }
    let matching: Vec<AgentId> = g
        .casts
        .iter()
        .filter(|(skill, _, start, end)| {
            ids_to_check.contains(skill) && *start <= time && time <= *end + SERVER_DELAY
        })
        .map(|(_, caster, _, _)| *caster)
        .collect();
    match matching.len() {
        0 => no_cast_src_finder(ranger, dst),
        1 => {
            // ranger Uncertain（Essence of Speed 等歧义）→ unknown
            if ranger == Certainty::Uncertain {
                None
            } else {
                Some(matching[0])
            }
        }
        _ => None,
    }
}

/// base GetIDs（BuffSourceFinder.cs:119-129）：DurationToIDs = {3000: {SoI,
/// TN, SandSquall}, 2000: {TN}}（20190305 表）。DurationCap==0 → ±15 精确
/// 匹配；否则 extension ≤ key+15 的并集（20210511 覆写）。
fn get_ids(g: &Globals, buff_id: i64, extension: i64, duration_cap_zero: bool) -> BTreeSet<i64> {
    let _ = (g, buff_id);
    let table: [(i64, &[i64]); 2] = [(3000, &[SOI, TN, SAND_SQUALL]), (2000, &[TN])];
    let mut out = BTreeSet::new();
    for (key, ids) in table {
        let hit = if duration_cap_zero {
            (extension - key).abs() <= FINDER_TOLERANCE
        } else {
            extension <= key + FINDER_TOLERANCE
        };
        if hit {
            out.extend(ids.iter().copied());
        }
    }
    out
}

/// NoCastSrcFinder（BuffSourceFinder.cs:131-144 现代退化）：ranger Uncertain
///（技能表命中但无 cast）→ dst（Essence of Speed / Resounding Timbre 自施）；
/// 其余 unknown。
fn no_cast_src_finder(ranger: Certainty, dst: AgentId) -> Option<AgentId> {
    if ranger == Certainty::Uncertain {
        Some(dst)
    } else {
        None
    }
}

// ===== 4. 清洗 + 注入 + 仿真（BuffDictionary.cs + SimulateBuffsAnd
// ComputeGraphs 的仿真面）=====

/// BuffsContainer.cs:196-253 的 stack-duration band-aid：HasStackIDs 且
/// Stacking/SCL 类型时，把「按 instance 序列重算的完整时长 == 移除值」的
/// RemoveSingle 的 RemovedDuration 改写为剩余时长（−activeTime − elapsed），
/// 使仿真层的 ±15ms 近似匹配成立。位置：BuffsContainer ctor（ParsedEvtcLog
/// 内，TryFindSrc 前）→ 在 dedup/注入前对桶行原地改写。
#[allow(clippy::ptr_arg)]
fn band_aid_stack_removals(ctx: &Ctx<'_>, rows: &mut Vec<Row>, has_stack_ids: bool) {
    if !has_stack_ids {
        return;
    }
    // 按 buff id 分组（行已按流序 = 时间序）
    let mut by_buff: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_buff.entry(r.buff_id).or_default().push(i);
    }
    for (buff_id, idxs) in by_buff {
        let Some(def) = ctx.registry.get(buff_id) else { continue };
        let Some(stack) = parse_stack_type(&def.stack_type) else { continue };
        let is_scl = stack == BuffStackType::StackingConditionalLoss;
        let is_stacking = stack == BuffStackType::Stacking;
        if !is_scl && !is_stacking {
            continue;
        }
        let group_rows: Vec<&Row> = idxs.iter().map(|&i| &rows[i]).collect();
        // 触发条件：存在合规 RemoveSingle（SCL 或 removed == int.MaxValue）
        let any_remove = group_rows.iter().any(|r| {
            matches!(
                r.kind,
                RowKind::RemoveSingle {
                    removed,
                    overstack_or_natural_end: false,
                    ..
                } if is_scl || removed == i64::from(i32::MAX)
            )
        });
        if !any_remove {
            continue;
        }
        // 分桶（instance → 行 idx 列表）
        let mut applies: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        let mut others: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        let mut removes: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for &i in &idxs {
            match rows[i].kind {
                RowKind::Apply { instance, .. } if instance != 0 => {
                    applies.entry(instance).or_default().push(i);
                }
                RowKind::Ext { instance, .. } if instance != 0 => {
                    others.entry(instance).or_default().push(i);
                }
                RowKind::StackActive { instance } | RowKind::StackDeactive { instance, .. }
                    if instance != 0 =>
                {
                    others.entry(instance).or_default().push(i);
                }
                RowKind::RemoveSingle {
                    instance,
                    removed,
                    overstack_or_natural_end: false,
                } if instance != 0 && (is_scl || removed == i64::from(i32::MAX)) => {
                    removes.entry(instance).or_default().push(i);
                }
                _ => {}
            }
        }
        for (instance, remove_idxs) in removes {
            let Some(apply_idxs) = applies.get(&instance) else { continue };
            let others_idxs = others.get(&instance).cloned().unwrap_or_default();
            for &r_idx in &remove_idxs {
                let remove_time = rows[r_idx].time;
                let Some(&a_idx) = apply_idxs
                    .iter()
                    .rev()
                    .find(|&&a| rows[a].time <= remove_time)
                else {
                    continue;
                };
                // 重算 total：original + Σ ext − Σ active 段
                let RowKind::Apply {
                    applied,
                    original,
                    ..
                } = rows[a_idx].kind
                else {
                    unreachable!()
                };
                let mut total_duration = original;
                let mut previous_time = rows[a_idx].time;
                for &o_idx in &others_idxs {
                    let o = &rows[o_idx];
                    if o.time >= rows[a_idx].time && o.time <= remove_time {
                        match o.kind {
                            RowKind::Ext { ext_dur, .. } => {
                                total_duration += ext_dur;
                            }
                            RowKind::StackActive { .. } => {
                                total_duration -= o.time - previous_time;
                            }
                            _ => {}
                        }
                    }
                    previous_time = o.time;
                }
                let RowKind::RemoveSingle { removed, .. } = rows[r_idx].kind else {
                    unreachable!()
                };
                if total_duration == removed {
                    let active_time = original - applied;
                    let elapsed = remove_time - rows[a_idx].time;
                    let rewritten = (removed - active_time - elapsed).max(0);
                    if let RowKind::RemoveSingle { removed, .. } = &mut rows[r_idx].kind {
                        *removed = rewritten;
                    }
                }
            }
        }
    }
}



#[allow(clippy::too_many_arguments)]
fn cleanse_and_simulate(
    ctx: &Ctx<'_>,
    g: &Globals,
    actor: AgentId,
    rows: Vec<Row>,
    despawn_times: Vec<i64>,
    spawn_times: Vec<i64>,
    active_duration: i64,
) -> ActorSims {
    let log = ctx.log;
    let has_stack_ids = log
        .events
        .iter()
        .any(|e| matches!(e, CombatEvent::BuffStackActive(_) | CombatEvent::BuffStackDeactive(_)));
    struct Feed {
        rows: Vec<Row>,
        /// instance → (feed 内 idx, new_dur, old_dur, time)（dedup 状态）
        ext_state: BTreeMap<u32, (usize, i64, i64, i64)>,
        last_removed_regen: Option<(i64, i64, u32)>,
        def: BuffDef,
        stack: BuffStackType,
    }
    // ---- Band-aid（BuffsContainer.cs:196-253）：HasStackIDs 时对
    // Stacking/SCL buff 的 RemoveSingle 时长改写（让近似匹配成立）----
    let mut rows = rows;
    band_aid_stack_removals(ctx, &mut rows, has_stack_ids);
    let mut feeds: BTreeMap<i64, Feed> = BTreeMap::new();
    for row in rows {
        let Some(def) = ctx.registry.get(row.buff_id) else {
            continue; // ComputeBuffMap 的注册表门控
        };
        let def = def.clone();
        let Some(stack) = parse_stack_type(&def.stack_type) else {
            continue; // Unknown stack type 不仿真（C# CreateSimulator 会抛）
        };
        let is_regen = stack == BuffStackType::Regeneration;
        let feed = feeds.entry(row.buff_id).or_insert_with(|| Feed {
            rows: Vec::new(),
            ext_state: BTreeMap::new(),
            last_removed_regen: None,
            def: def.clone(),
            stack,
        });
        // ---- compliance（IsBuffSimulatorCompliant；NoID 面）----
        let compliant = match &row.kind {
            RowKind::Apply { .. } => true,
            RowKind::Ext { new_dur, .. } => *new_dur >= 0,
            RowKind::RemoveSingle {
                overstack_or_natural_end,
                ..
            } => !overstack_or_natural_end,
            RowKind::RemoveAll => true,
            RowKind::StackActive { instance } => *instance != 0 && row.buff_id == REGEN_BUFF_ID,
            RowKind::StackDeactive { .. } => false,
        };
        let mut row = row;
        if is_regen {
            // AddRegen（BuffDictionary.cs:98-119）
            if !compliant {
                if let RowKind::RemoveSingle {
                    instance,
                    removed,
                    ..
                } = row.kind
                    && has_stack_ids
                    && removed > sim::REMOVE_TOLERANCE
                {
                    feed.last_removed_regen = Some((row.time, removed, instance));
                }
                continue;
            }
            if let Some((t, removed, instance)) = feed.last_removed_regen
                && let RowKind::Apply { .. } = &mut row.kind
            {
                if row.time - t < SERVER_DELAY
                    && let RowKind::Apply { regen_override, .. } = &mut row.kind
                {
                    *regen_override = Some((removed, u64::from(instance)));
                }
                feed.last_removed_regen = None;
            }
        } else if !compliant {
            continue;
        }
        // ---- dedup（BuffDictionary.AddToList:21-79）----
        enum Dedup {
            Keep,
            Drop,
            Replace(usize),
        }
        let (decision, ext_reg): (Dedup, Option<(u32, i64, i64)>) = match &row.kind {
            RowKind::Ext {
                instance,
                new_dur,
                ext_dur,
                ..
            } => {
                if *ext_dur < 1 {
                    (Dedup::Drop, None)
                } else if *instance == 0 {
                    (Dedup::Keep, None)
                } else {
                    let cur_old = *new_dur - *ext_dur;
                    match feed.ext_state.get(instance) {
                        None => (Dedup::Keep, Some((*instance, *new_dur, cur_old))),
                        Some(&(idx, last_new, last_old, last_time)) => {
                            if (row.time - last_time).abs() <= 1 {
                                if (cur_old - last_old).abs() <= 1 {
                                    // 替换 list[last.index]（保位置）
                                    (Dedup::Replace(idx), Some((*instance, *new_dur, cur_old)))
                                } else if (*new_dur - last_new).abs() <= 1 {
                                    (Dedup::Drop, None)
                                } else {
                                    (Dedup::Keep, Some((*instance, *new_dur, cur_old)))
                                }
                            } else {
                                (Dedup::Keep, Some((*instance, *new_dur, cur_old)))
                            }
                        }
                    }
                }
            }
            RowKind::Apply {
                instance,
                initial,
                ..
            } => {
                if *instance != 0 && *initial {
                    // BuffInitial 全局判重（:65-73）
                    let dup = g
                        .applies_by_inst
                        .get(&(row.buff_id as u32, *instance))
                        .is_some_and(|list| {
                            list.iter().any(|a| !a.initial && (a.time - row.time).abs() <= 1)
                        });
                    if dup {
                        (Dedup::Drop, None)
                    } else {
                        (Dedup::Keep, None)
                    }
                } else {
                    (Dedup::Keep, None)
                }
            }
            _ => (Dedup::Keep, None),
        };
        match decision {
            Dedup::Drop => {}
            Dedup::Replace(idx) => {
                if let Some((instance, new_dur, cur_old)) = ext_reg {
                    feed.ext_state.insert(instance, (idx, new_dur, cur_old, row.time));
                }
                feed.rows[idx] = row;
            }
            Dedup::Keep => {
                if let Some((instance, new_dur, cur_old)) = ext_reg {
                    feed.ext_state
                        .insert(instance, (feed.rows.len(), new_dur, cur_old, row.time));
                }
                feed.rows.push(row);
            }
        }
    }
    // ---- Finalize（BuffDictionary.cs:122-157）：注入 + 稳定排序 ----
    let agent_item = log.agents.slot(actor).expect("actor slot");
    let (first_aware, last_aware) = (agent_item.first_aware, agent_item.last_aware);
    let log_end = log.log_data.evtc_log_end;
    for feed in feeds.values_mut() {
        let mut last_despawn = first_aware;
        for &t in &despawn_times {
            last_despawn = t;
            feed.rows.push(synthetic_remove_all(t + SERVER_DELAY, feed.def.id));
        }
        if last_aware < log_end - 2000 && last_aware - last_despawn > 2000 {
            feed.rows
                .push(synthetic_remove_all(last_aware + SERVER_DELAY, feed.def.id));
        }
        for &t in &spawn_times {
            feed.rows.push(synthetic_remove_all(t - SERVER_DELAY, feed.def.id));
        }
        feed.rows.sort_by_key(|r| r.time); // SortByTime 稳定
    }
    // ---- 仿真（SimulateBuffsAndComputeGraphs:976-1003 的逐 buff 面）----
    let mut by_buff: BTreeMap<i64, OneBuff> = BTreeMap::new();
    let log_start = log.log_data.log_start;
    let healing_of = |key: AgentId| -> u16 {
        if key == NO_AGENT {
            0
        } else {
            log.agents.slot(key).map(|a| a.healing).unwrap_or(0)
        }
    };
    for (buff_id, feed) in &feeds {
        if feed.rows.is_empty() {
            continue;
        }
        // 容量：BuffInfo MaxStacks 覆盖（Buff.cs:198-215）
        let mut capacity = feed.def.capacity;
        if let Some(evt) = log.metadata.buff_info_by_id.get(&(*buff_id as u32))
            && let Some(info) = evt.info.as_ref()
            && info.max_stacks > 0
            && i64::from(info.max_stacks) != capacity
        {
            capacity = i64::from(info.max_stacks);
        }
        let mut sim = Simulator::new(feed.stack, capacity, &healing_of);
        let cmds: Vec<SimCmd<AgentId>> = feed
            .rows
            .iter()
            .map(|r| row_to_cmd(r, feed.stack))
            .collect();
        sim.simulate(&cmds, log_start, log_end);
        let intensity = matches!(
            feed.stack,
            BuffStackType::Stacking
                | BuffStackType::StackingUniquePerSrc
                | BuffStackType::StackingConditionalLoss
        );
        let distrib = stats::phase_distrib(&sim.result, log_start, log_end);
        by_buff.insert(
            *buff_id,
            OneBuff {
                buff: feed.def.clone(),
                result: sim.result,
                distrib,
                intensity,
            },
        );
    }
    ActorSims {
        by_buff,
        active_duration,
    }
}

fn synthetic_remove_all(time: i64, buff_id: i64) -> Row {
    Row {
        ev: usize::MAX,
        time,
        by: NO_AGENT,
        buff_id,
        kind: RowKind::RemoveAll,
    }
}

fn row_to_cmd(r: &Row, stack: BuffStackType) -> SimCmd<AgentId> {
    match r.kind {
        RowKind::Apply {
            instance,
            applied,
            added_active,
            regen_override,
            ..
        } => {
            let (overriden_duration, overriden_stack_id) = regen_override.unwrap_or((0, 0));
            // BuffApplyEvent.UpdateSimulator：addedActive = _addedActive ||
            // (forceActive && StackType == StackingConditionalLoss)
            let added = added_active || stack == BuffStackType::StackingConditionalLoss;
            SimCmd::Add {
                duration: applied,
                src: r.by,
                start: r.time,
                stack_id: u64::from(instance),
                added_active: added,
                overriden_duration,
                overriden_stack_id,
            }
        }
        RowKind::Ext {
            instance,
            new_dur,
            ext_dur,
            ..
        } => SimCmd::Extend {
            extension: ext_dur,
            old_value: new_dur - ext_dur,
            src: r.by,
            time: r.time,
            stack_id: u64::from(instance),
        },
        RowKind::RemoveSingle { removed, .. } => SimCmd::Remove {
            removed_duration: removed,
            time: r.time,
            all: false,
        },
        RowKind::RemoveAll => SimCmd::Remove {
            removed_duration: 0,
            time: r.time,
            all: true,
        },
        RowKind::StackActive { instance } => SimCmd::Activate {
            time: r.time,
            stack_id: u64::from(instance),
        },
        RowKind::StackDeactive { .. } => unreachable!("deactive never feeds sim"),
    }
}

