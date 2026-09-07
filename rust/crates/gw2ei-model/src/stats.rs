//! P2a 事件索引 + A 级统计聚合器。
//!
//! 对齐 `CombatDataFetchers.cs`（per-agent 桶）+ `EIData/Statistics/` +
//! `SingleActor{Status,Graphs}Helper` 的 A 级（纯事件）聚合面。
//!
//! 索引方式：对已事件化（地址已重写）的事件单遍扫描，把 `(address, time)` 解析
//! 为 AgentId 后按角色分桶（伤害/承伤/破蔑/控场/晕断/施法/武器切换/状态），
//! 等价 C# `CombatData` 构造器的 GroupBy 分桶（CombatData.cs:667-684）。
//!
//! **Stub 标记（StatsCompleteness）**：依赖 Buff 注册表/Sim/技能内容表/
//! combat replay 位置的字段本阶段显式输出 0/默认值并标记，不静默伪装成真值：
//! - `condi/power 轴`（非直伤行的条件/威力分流按 buff 分类，注册表在 P3）
//! - `critableDirectDamageCount`（NonCritableSkills 覆盖表，P3）
//! - `avgBoons/avgConditions(Active)`（BuffSimulator，P3）
//! - `skillCastUptime(NoAA)`（技能 AA 分类，P3）；`wasted/saved` 已真算
//! - `distanceToCenterOfSquad/distanceToCommander`（combat replay 位置，P2c）
//! - `BoonStrips/ConditionCleanses(+time)` 与 Support 的同类（buff 分类，P3）
//! - buffVolumes / boonsStates / conditionsStates（buff 注册表 + Sim，P3）
//! - DownContribution 走现代路径（`IsDownedBeforeNext90`；Last90 事件已于 20240529
//!   退役）—— 事件纯算法，真算。

use std::collections::BTreeMap;

use crate::agent::{AgentId, AgentTable, NO_AGENT};
use crate::events::CombatEvent;
use crate::parsed::ParsedLog;

/// 行级事件按角色索引（= CombatDataFetchers 的字典面）。
#[derive(Debug, Clone, Default)]
pub struct EventIndex {
    /// From 侧健康伤害（Direct/NonDirect/NoDamage 全含）。
    pub damage_from: BTreeMap<AgentId, Vec<usize>>,
    pub damage_to: BTreeMap<AgentId, Vec<usize>>,
    pub breakbar_damage_from: BTreeMap<AgentId, Vec<usize>>,
    pub breakbar_damage_to: BTreeMap<AgentId, Vec<usize>>,
    pub cc_from: BTreeMap<AgentId, Vec<usize>>,
    pub cc_to: BTreeMap<AgentId, Vec<usize>>,
    /// StunBreak 收到侧（To=被打断者；From 恒 unknown —— 与 C# 一致）。
    pub stun_break_to: BTreeMap<AgentId, Vec<usize>>,
    /// 施法（AnimatedCast 族）按 caster。
    pub cast_by_caster: BTreeMap<AgentId, Vec<usize>>,
    pub weapon_swap_by_caster: BTreeMap<AgentId, Vec<usize>>,
    // 状态事件按 src
    pub down: BTreeMap<AgentId, Vec<usize>>,
    pub alive: BTreeMap<AgentId, Vec<usize>>,
    pub dead: BTreeMap<AgentId, Vec<usize>>,
    pub spawn: BTreeMap<AgentId, Vec<usize>>,
    pub despawn: BTreeMap<AgentId, Vec<usize>>,
    pub health_update: BTreeMap<AgentId, Vec<usize>>,
    pub barrier_update: BTreeMap<AgentId, Vec<usize>>,
    pub max_health_update: BTreeMap<AgentId, Vec<usize>>,
    pub enter_combat: BTreeMap<AgentId, Vec<usize>>,
    pub exit_combat: BTreeMap<AgentId, Vec<usize>>,
    pub breakbar_percent: BTreeMap<AgentId, Vec<usize>>,
    /// 是否有任何 breakbar 伤害（C# `HasBreakbarDamageData`）。
    pub has_breakbar_damage: bool,
    /// 是否有任何控场（C# `HasCrowdControlData`）。
    pub has_cc: bool,
    /// 有多少事件索引到 NO_AGENT（可观测性）。
    pub unresolved: u64,
}

/// 聚合完整性标记（P2a 输出审计；不参与数值语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsCompleteness {
    pub requires_buff_registry: bool,
    pub requires_simulator: bool,
    pub requires_skill_content: bool,
    pub requires_combat_replay: bool,
    /// 被「分类轴未知」排除的非直伤事件计数（P3 前它们既不进 condi 也不进
    /// power 小计 —— C# 注册表恒有分类，此计数应为 0）。
    pub unclassified_indirect_events: u64,
}

const UNCLASSIFIED: Option<bool> = None;

/// 伤害事件视图（聚合器的输入行）。
#[derive(Debug, Clone, Copy)]
pub struct DmgRow {
    pub time: i64,
    pub from: AgentId,
    pub to: AgentId,
    pub skill_id: u32,
    pub health_damage: i32,
    pub shield_damage: i32,
    pub is_non_direct: bool,
    pub is_no_damage: bool,
    pub is_life_leech: bool,
    pub has_hit: bool,
    pub has_crit: bool,
    pub has_glanced: bool,
    pub is_blind: bool,
    pub is_absorbed: bool,
    pub is_blocked: bool,
    pub is_evaded: bool,
    pub has_interrupted: bool,
    pub has_downed: bool,
    pub has_killed: bool,
    pub against_downed: bool,
    pub against_moving: bool,
    pub is_flanking: bool,
    pub is_over_ninety: bool,
    /// 目标相对来源的 IFF（C# `ToFriendly` 过滤用）。
    pub to_friendly: bool,
    /// 分类轴：Some(true)=条件伤 / Some(false)=威力伤 / None=注册表未载（Stub）。
    pub condition_based: Option<bool>,
}

pub fn build_event_index(log: &ParsedLog) -> EventIndex {
    let mut idx = EventIndex::default();
    let mut table = log.agents.clone();
    for (i, evt) in log.events.iter().enumerate() {
        let (a, b) = evt.participants();
        let (ra, rb) = (resolve(&mut table, a, evt.time()), resolve(&mut table, b, evt.time()));
        if a != 0 && ra == NO_AGENT {
            idx.unresolved += 1;
        }
        match evt {
            CombatEvent::DirectHealthDamage(_)
            | CombatEvent::NonDirectHealthDamage(_)
            | CombatEvent::NoDamageHealthDamage(_) => {
                push(&mut idx.damage_from, ra, i);
                push(&mut idx.damage_to, rb, i);
            }
            CombatEvent::BreakbarDamage(_) | CombatEvent::BreakbarRecovery(_) => {
                idx.has_breakbar_damage = true;
                push(&mut idx.breakbar_damage_from, ra, i);
                push(&mut idx.breakbar_damage_to, rb, i);
            }
            CombatEvent::CrowdControl(_) => {
                idx.has_cc = true;
                push(&mut idx.cc_from, ra, i);
                push(&mut idx.cc_to, rb, i);
            }
            CombatEvent::StunBreak(_) => {
                push(&mut idx.stun_break_to, ra, i);
            }
            CombatEvent::AnimatedCast(_)
            | CombatEvent::Emote(_)
            | CombatEvent::GadgetInteract(_)
            | CombatEvent::BundlePickUp(_) => {
                push(&mut idx.cast_by_caster, ra, i);
            }
            CombatEvent::WeaponSwap(_) => {
                push(&mut idx.weapon_swap_by_caster, ra, i);
            }
            CombatEvent::Down(_) => push(&mut idx.down, ra, i),
            CombatEvent::Alive(_) => push(&mut idx.alive, ra, i),
            CombatEvent::Dead(_) => push(&mut idx.dead, ra, i),
            CombatEvent::Spawn(_) => push(&mut idx.spawn, ra, i),
            CombatEvent::Despawn(_) => push(&mut idx.despawn, ra, i),
            CombatEvent::HealthUpdate(_) => push(&mut idx.health_update, ra, i),
            CombatEvent::BarrierUpdate(_) => push(&mut idx.barrier_update, ra, i),
            CombatEvent::MaxHealthUpdate(_) => push(&mut idx.max_health_update, ra, i),
            CombatEvent::EnterCombat(_) => push(&mut idx.enter_combat, ra, i),
            CombatEvent::ExitCombat(_) => push(&mut idx.exit_combat, ra, i),
            CombatEvent::BreakbarPercent(_) => push(&mut idx.breakbar_percent, ra, i),
            _ => {}
        }
    }
    // EIMetaAndStatusParse（CombatData.cs:376-430）：杀/倒事件与状态流事件
    // ±500ms 去重合成（仅非 unidentified-species 的承伤 agent）。
    let agents_snapshot: Vec<(AgentId, bool)> = idx
        .damage_to
        .keys()
        .filter(|&&k| k != NO_AGENT)
        .map(|&k| (k, table.slot(k).map(|a| a.is_non_identified_species()).unwrap_or(true)))
        .collect();
    for (agent, skip) in agents_snapshot {
        if skip {
            continue;
        }
        let mut to_add_dead: Vec<(i64, usize)> = Vec::new();
        let mut to_add_down: Vec<(i64, usize)> = Vec::new();
        if let Some(list) = idx.damage_to.get(&agent) {
            for &evi in list {
                let evt = &log.events[evi];
                let (time, has_killed, has_downed) = damage_flags(evt);
                if has_killed {
                    let dup = idx
                        .dead
                        .get(&agent)
                        .map(|l| l.iter().any(|&e| (log.events[e].time().unwrap_or(0) - time).abs() < 500))
                        .unwrap_or(false);
                    if !dup {
                        to_add_dead.push((time, evi));
                    }
                }
                if has_downed {
                    let dup = idx
                        .down
                        .get(&agent)
                        .map(|l| l.iter().any(|&e| (log.events[e].time().unwrap_or(0) - time).abs() < 500))
                        .unwrap_or(false);
                    if !dup {
                        to_add_down.push((time, evi));
                    }
                }
            }
        }
        if !to_add_dead.is_empty() {
            let e = idx.dead.entry(agent).or_default();
            e.extend(to_add_dead.iter().map(|&(_, i)| i));
            e.sort_by_key(|&i| log.events[i].time());
        }
        if !to_add_down.is_empty() {
            let e = idx.down.entry(agent).or_default();
            e.extend(to_add_down.iter().map(|&(_, i)| i));
            e.sort_by_key(|&i| log.events[i].time());
        }
    }
    idx
}

fn resolve(table: &mut AgentTable, addr: u64, time: Option<i64>) -> AgentId {
    match time {
        Some(t) => table.resolve_agent(addr, t),
        None if addr != 0 => table
            .all_ids()
            .into_iter()
            .find(|&id| table.slot(id).expect("slot must exist").address == addr)
            .unwrap_or(NO_AGENT),
        None => NO_AGENT,
    }
}

fn push(map: &mut BTreeMap<AgentId, Vec<usize>>, k: AgentId, v: usize) {
    if k != NO_AGENT {
        map.entry(k).or_default().push(v);
    }
}

fn damage_flags(evt: &CombatEvent) -> (i64, bool, bool) {
    use CombatEvent::*;
    let f = match evt {
        DirectHealthDamage(f) | NonDirectHealthDamage(f) | NoDamageHealthDamage(f) => Some(f),
        _ => None,
    };
    match f {
        Some(f) => (f.skill.time, f.has_killed, f.has_downed),
        None => (0, false, false),
    }
}

/// 行视图：把事件索引行转成 DmgRow（供聚合）。
pub fn dmg_row(evt: &CombatEvent, from: AgentId, to: AgentId) -> Option<DmgRow> {
    use CombatEvent::*;
    match evt {
        DirectHealthDamage(f) => Some(DmgRow {
            time: f.skill.time,
            from,
            to,
            skill_id: f.skill.skill_id,
            health_damage: f.health_damage,
            shield_damage: f.shield_damage,
            is_non_direct: false,
            is_no_damage: f.is_not_a_damage_event,
            is_life_leech: false,
            has_hit: f.has_hit,
            has_crit: f.has_crit,
            has_glanced: f.has_glanced,
            is_blind: f.is_blind,
            is_absorbed: f.is_absorbed,
            is_blocked: f.is_blocked,
            is_evaded: f.is_evaded,
            has_interrupted: f.has_interrupted,
            has_downed: f.has_downed,
            has_killed: f.has_killed,
            against_downed: f.skill.against_downed,
            against_moving: f.skill.against_moving,
            is_flanking: f.skill.is_flanking,
            is_over_ninety: f.skill.is_over_ninety,
            to_friendly: f.skill.iff == gw2ei_parse::Iff::Friend,
            condition_based: Some(false),
        }),
        NonDirectHealthDamage(f) => {
            // 分类轴：非直伤行按 buff 注册表分类 —— P2a 无注册表 → None（Stub）
            let _ = UNCLASSIFIED;
            Some(DmgRow {
                time: f.skill.time,
                from,
                to,
                skill_id: f.skill.skill_id,
                health_damage: f.health_damage,
                shield_damage: f.shield_damage,
                is_non_direct: true,
                is_no_damage: f.is_not_a_damage_event,
                is_life_leech: f.is_life_leech,
                has_hit: f.has_hit,
                has_crit: f.has_crit,
                has_glanced: f.has_glanced,
                is_blind: f.is_blind,
                is_absorbed: f.is_absorbed,
                is_blocked: f.is_blocked,
                is_evaded: f.is_evaded,
                has_interrupted: f.has_interrupted,
                has_downed: f.has_downed,
                has_killed: f.has_killed,
                against_downed: f.skill.against_downed,
                against_moving: f.skill.against_moving,
                is_flanking: f.skill.is_flanking,
                is_over_ninety: f.skill.is_over_ninety,
                to_friendly: f.skill.iff == gw2ei_parse::Iff::Friend,
                condition_based: None,
            })
        }
        NoDamageHealthDamage(f) => Some(DmgRow {
            time: f.skill.time,
            from,
            to,
            skill_id: f.skill.skill_id,
            health_damage: f.health_damage,
            shield_damage: f.shield_damage,
            is_non_direct: false,
            is_no_damage: true,
            is_life_leech: false,
            has_hit: false,
            has_crit: false,
            has_glanced: false,
            is_blind: false,
            is_absorbed: false,
            is_blocked: false,
            is_evaded: false,
            has_interrupted: f.has_interrupted,
            has_downed: f.has_downed,
            has_killed: f.has_killed,
            against_downed: false,
            against_moving: false,
            is_flanking: false,
            is_over_ninety: false,
            to_friendly: false,
            condition_based: Some(false),
        }),
        _ => None,
    }
}

/// 无 Sim 依赖的聚合输出结构。字段命名对齐 JsonStatistics 内嵌 DTO
///（camelCase 语义在 P2b JSON 层做）。
#[derive(Debug, Clone, Default)]
pub struct DamageStats {
    pub dps: i32,
    pub damage: i32,
    pub condi_dps: i32,
    pub condi_damage: i32,
    pub power_dps: i32,
    pub power_damage: i32,
    pub strike_dps: i32,
    pub strike_damage: i32,
    pub life_leech_dps: i32,
    pub life_leech_damage: i32,
    pub breakbar_damage: f64,
    pub actor_dps: i32,
    pub actor_damage: i32,
    pub actor_condi_dps: i32,
    pub actor_condi_damage: i32,
    pub actor_power_dps: i32,
    pub actor_power_damage: i32,
    pub actor_strike_dps: i32,
    pub actor_strike_damage: i32,
    pub actor_life_leech_dps: i32,
    pub actor_life_leech_damage: i32,
    pub actor_breakbar_damage: f64,
    pub completeness: StatsCompleteness,
}

/// C# `ComputeDamageFrom`（DamageStatistics.cs:65-99）的纯函数。
#[allow(clippy::type_complexity)]
pub fn compute_damage_from(rows: &[DmgRow]) -> (i32, i32, i32, i32, i32, i32, u64) {
    let (mut all, mut power, mut condi, mut strike, mut life_leech, mut barrier) = (0, 0, 0, 0, 0, 0);
    let mut unknown = 0u64;
    for d in rows {
        all += d.health_damage;
        if d.is_non_direct {
            match d.condition_based {
                Some(true) => condi += d.health_damage,
                Some(false) => {
                    power += d.health_damage;
                    if d.is_life_leech {
                        life_leech += d.health_damage;
                    }
                }
                None => unknown += 1,
            }
        } else {
            strike += d.health_damage;
            power += d.health_damage;
        }
        barrier += d.shield_damage;
    }
    (all, power, condi, strike, life_leech, barrier, unknown)
}

fn round(x: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    (x * f).round_ties_even() / f
}
// ===== Actor 状态/序列（SingleActorStatusHelper + GraphsHelper 面）=====

/// 状态段（Segment.cs 语义：Start/End/Value；面积 = (End-Start)*Value）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: i64,
    pub end: i64,
    pub value: f64,
}

impl Segment {
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
    /// `IntersectingArea`（SegmentExt.cs）：`max(minEnd-maxStart,0) * value`。
    pub fn intersecting_area(&self, start: i64, end: i64) -> f64 {
        let s = self.start.max(start);
        let e = self.end.min(end);
        (e - s).max(0) as f64 * self.value
    }
}

/// 状态段计算（GetAgentStatus + FillStatus，SingleActorStatusHelper.cs:37-156）。
/// kind: 0=Down, 1=Dead, 2=Despawn, 3=Alive/Spawn/其他（→ active）。
pub fn status_segments(
    events: &[(i64, u8)],
    first_aware: i64,
    last_aware: i64,
) -> (Vec<Segment>, Vec<Segment>, Vec<Segment>, Vec<Segment>) {
    let mut dead = Vec::new();
    let mut down = Vec::new();
    let mut dc = Vec::new();
    let mut actives = Vec::new();
    let add = |seg: &mut Vec<Segment>, start: i64, end: i64| {
        if start < end {
            seg.push(Segment { start, end, value: 1.0 });
        }
    };
    dc.push(Segment { start: i64::MIN, end: first_aware, value: 1.0 });
    if events.is_empty() {
        actives.push(Segment { start: first_aware, end: last_aware, value: 1.0 });
        dc.push(Segment { start: last_aware, end: i64::MAX, value: 1.0 });
        return (dead, down, dc, actives);
    }
    for (i, &(time, kind)) in events.iter().enumerate() {
        let next_time = if i + 1 < events.len() { events[i + 1].0 } else { last_aware };
        match kind {
            0 => {
                if i == 0 {
                    add(&mut actives, first_aware, time);
                }
                add(&mut down, time, next_time);
            }
            1 => {
                if i == 0 {
                    add(&mut actives, first_aware, time);
                }
                add(&mut dead, time, next_time);
            }
            2 => {
                if i == 0 {
                    add(&mut actives, first_aware, time);
                }
                add(&mut dc, time, next_time);
            }
            _ => {
                if i == 0 && time - first_aware > 50 {
                    add(&mut dc, first_aware, time);
                }
                add(&mut actives, time, next_time);
            }
        }
    }
    if let Some(&(time, kind)) = events.last() {
        match kind {
            0 => add(&mut down, time, last_aware),
            1 => {
                add(&mut dead, time, last_aware);
                dead.push(Segment { start: last_aware, end: i64::MAX, value: 1.0 });
            }
            2 => add(&mut dc, time, last_aware),
            _ => add(&mut actives, time, last_aware),
        }
        if kind != 1 {
            dc.push(Segment { start: last_aware, end: i64::MAX, value: 1.0 });
        }
    }
    (dead, down, dc, actives)
}

/// `ListFromStates`（SingleActorGraphsHelper.cs:81-111）：状态事件 → 融合段。
pub fn list_from_states(states: &[(i64, f64)], log_start: i64, log_end: i64) -> Vec<Segment> {
    if states.is_empty() {
        return Vec::new();
    }
    let mut res: Vec<Segment> = Vec::with_capacity(states.len() + 1);
    let mut last_value = states[0].1;
    for &(start, state) in states {
        let end = start.clamp(log_start, log_end);
        if res.is_empty() {
            res.push(Segment { start: log_start, end, value: last_value });
        } else {
            let prev_end = res.last().expect("non-empty after first push").end;
            res.push(Segment { start: prev_end, end, value: last_value });
        }
        last_value = state;
    }
    let prev_end = res.last().expect("non-empty after first push").end;
    res.push(Segment { start: prev_end, end: log_end, value: last_value });
    res.retain(|s| !s.is_empty());
    let mut fused: Vec<Segment> = Vec::with_capacity(res.len());
    for seg in res {
        if let Some(last) = fused.last_mut()
            && last.value == seg.value {
                last.end = seg.end;
                continue;
            }
        fused.push(seg);
    }
    fused
}

/// 健康/屏障百分比更新点列（GetHealthUpdates + JsonActorBuilder.cs:100-101）。
pub fn percent_points(
    events: &[(i64, f64)],
    log_start: i64,
    log_end: i64,
) -> Vec<(i64, f64)> {
    list_from_states(events, log_start, log_end)
        .into_iter()
        .map(|seg| (seg.start, seg.value))
        .collect()
}

/// 1s 伤害图（ComputeDamageGraph，SingleActorGraphsHelper.cs:113-139）：
/// 桶数 = (end-start)/1000 整除时 +1，否则 +2（InterpolatedGraph.cs:20）。
/// rows 按时间升序的 (time, damage)。
pub fn damage_graph_1s(rows: &[(i64, i64)], start: i64, end: i64) -> Vec<i64> {
    let duration_ms = end - start;
    let duration_s = duration_ms / 1000;
    let len = if duration_s * 1000 != duration_ms { duration_s + 2 } else { duration_s + 1 };
    let mut values = vec![0i64; len as usize];
    let mut previous_time = 0i64;
    for &(time, dmg) in rows {
        let bucket = ((time - start) as f64 / 1000.0).ceil() as i64;
        if bucket != previous_time {
            // C# `for i in previousTime+1..=time` —— **含当前桶**（该桶先继承
            // 上一桶的累计值再 += 本秒伤害）→ 图是「每秒末累计伤害」。
            let fill = values[previous_time as usize];
            for slot in values
                .iter_mut()
                .take(bucket as usize + 1)
                .skip(previous_time as usize + 1)
            {
                *slot = fill;
            }
        }
        previous_time = bucket;
        values[bucket as usize] += dmg;
    }
    let tail_fill = values[previous_time as usize];
    for slot in values.iter_mut().skip(previous_time as usize + 1) {
        *slot = tail_fill;
    }
    values
}

/// IsDownedBeforeNext90（SingleActorStatusHelper.cs:404-438）—— 现代
/// DownContribution 判据。
pub fn is_down_before_next_90(
    downs: &[Segment],
    health_points: &[(i64, f64)],
    cur_time: i64,
    log_end: i64,
    is_down_at: bool,
) -> bool {
    if is_down_at {
        return false;
    }
    let hp_at = |t: i64| -> f64 {
        health_points
            .iter()
            .filter(|&&(st, _)| st <= t)
            .map(|&(_, v)| v)
            .next_back()
            .unwrap_or(-1.0)
    };
    if hp_at(cur_time) > 90.0 {
        return false;
    }
    let next_down = downs
        .iter()
        .find(|d| d.intersecting_area(cur_time, log_end) > 0.0);
    let Some(next_down) = next_down else { return false };
    let next_90 = health_points
        .iter()
        .filter(|&&(st, v)| st > cur_time && v > 90.0)
        .map(|&(st, _)| st)
        .next();
    match next_90 {
        None => true,
        Some(t) => t >= next_down.start,
    }
}

// ===== 伤害统计（DamageStatistics.cs:37-63）=====

/// 计算 actor 的 DPS 统计块（JsonDPS 14 键语义；condi/power 分类轴 Stub）。
#[allow(clippy::too_many_arguments)]
pub fn compute_dps_stats(
    actor_out: &[DmgRow],
    actor_only: &[DmgRow],
    breakbar_out: f64,
    breakbar_actor: f64,
    start: i64,
    end: i64,
) -> DamageStats {
    let phase_duration = (end - start) as f64 / 1000.0;
    let mut out = DamageStats::default();
    let (dmg, pw, cd, st, ll, _barrier, unk_all) = compute_damage_from(actor_out);
    let (dmg_a, pw_a, cd_a, st_a, ll_a, _b_a, unk_actor) = compute_damage_from(actor_only);
    out.damage = dmg;
    out.power_damage = pw;
    out.condi_damage = cd;
    out.strike_damage = st;
    out.life_leech_damage = ll;
    out.actor_damage = dmg_a;
    out.actor_power_damage = pw_a;
    out.actor_condi_damage = cd_a;
    out.actor_strike_damage = st_a;
    out.actor_life_leech_damage = ll_a;
    out.completeness = StatsCompleteness {
        unclassified_indirect_events: unk_all + unk_actor,
        requires_buff_registry: unk_all + unk_actor > 0,
        ..Default::default()
    };
    if phase_duration > 0.0 {
        out.dps = (out.damage as f64 / phase_duration).round_ties_even() as i32;
        out.power_dps = (out.power_damage as f64 / phase_duration).round_ties_even() as i32;
        out.condi_dps = (out.condi_damage as f64 / phase_duration).round_ties_even() as i32;
        out.strike_dps = (out.strike_damage as f64 / phase_duration).round_ties_even() as i32;
        out.life_leech_dps = (out.life_leech_damage as f64 / phase_duration).round_ties_even() as i32;
        out.actor_dps = (out.actor_damage as f64 / phase_duration).round_ties_even() as i32;
        out.actor_power_dps = (out.actor_power_damage as f64 / phase_duration).round_ties_even() as i32;
        out.actor_condi_dps = (out.actor_condi_damage as f64 / phase_duration).round_ties_even() as i32;
        out.actor_strike_dps = (out.actor_strike_damage as f64 / phase_duration).round_ties_even() as i32;
        out.actor_life_leech_dps = (out.actor_life_leech_damage as f64 / phase_duration).round_ties_even() as i32;
    }
    out.breakbar_damage = round(breakbar_out, 1);
    out.actor_breakbar_damage = round(breakbar_actor, 1);
    out
}

