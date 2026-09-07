//! 统计聚合:把行视图(C# Get*Events 面)折叠为 Json 统计 DTO 数值。
//! 语义对齐 EIData/Statistics/*.cs 与 JsonStatisticsBuilder。

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, EventIndex, NO_AGENT, ParsedLog, Segment};

use crate::ctx::Ctx;
use crate::dto_b::{
    JsonDps, JsonGameplayStats,
};
use crate::rows::{ActorRows, DmgLine, HealthLine};
use crate::skills::is_swap;

fn round(x: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    (x * f).round_ties_even() / f
}
/// C# Math.Round(整型中间参数) = banker's;ms → 秒 TimeDigit=3。
fn round3(x: f64) -> f64 {
    round(x / 1000.0, 3)
}

// ===== 分类注入的 DmgRow 视图 =====
/// 把 HealthLine 行转成可聚合行(分类按 Buff 注册表)。
pub fn row_view<'a>(ctx: &'a Ctx<'a>, hl: &HealthLine) -> Option<gw2ei_model::DmgRow> {
    let mut row = gw2ei_model::dmg_row(
        &ctx.log.events[hl.line.ev],
        hl.line.from,
        hl.line.to,
    )?;
    if row.is_non_direct {
        row.condition_based = Some(ctx.registry.is_condition(crate::signed_id(row.skill_id)));
    }
    Some(row)
}

// ===== JsonDPS(DamageStatistics.cs)=====
pub struct DamageAcc {
    pub damage: i64,
    pub power: i64,
    pub condi: i64,
    pub strike: i64,
    pub leech: i64,
    pub barrier: i64,
}

/// DamageStatistics.ComputeDamageFrom(全部行;hit 与否都计入 health)。
pub fn compute_damage_from(rows: impl Iterator<Item = gw2ei_model::DmgRow>) -> DamageAcc {
    let mut a = DamageAcc { damage: 0, power: 0, condi: 0, strike: 0, leech: 0, barrier: 0 };
    for d in rows {
        a.damage += i64::from(d.health_damage);
        if d.is_non_direct {
            if d.condition_based == Some(true) {
                a.condi += i64::from(d.health_damage);
            } else {
                a.power += i64::from(d.health_damage);
                if d.is_life_leech {
                    a.leech += i64::from(d.health_damage);
                }
            }
        } else {
            a.strike += i64::from(d.health_damage);
            a.power += i64::from(d.health_damage);
        }
        a.barrier += i64::from(d.shield_damage);
    }
    a
}

fn dps(v: i64, dur_s: f64) -> i64 {
    if dur_s > 0.0 {
        (v as f64 / dur_s).round_ties_even() as i64
    } else {
        0
    }
}

/// JsonDPS 构建:actor 全行 vs 仅 actor(无 minions)。
#[allow(clippy::too_many_arguments)]
pub fn build_dps(
    out_all: &[gw2ei_model::DmgRow],
    actor_rows: &[gw2ei_model::DmgRow],
    bb_all: f64,
    bb_actor: f64,
    start: i64,
    end: i64,
) -> JsonDps {
    let dur_s = (end - start) as f64 / 1000.0;
    let all = compute_damage_from(out_all.iter().copied());
    let act = compute_damage_from(actor_rows.iter().copied());
    JsonDps {
        dps: dps(all.damage, dur_s),
        damage: all.damage,
        condi_dps: dps(all.condi, dur_s),
        condi_damage: all.condi,
        power_dps: dps(all.power, dur_s),
        power_damage: all.power,
        breakbar_damage: round(bb_all, 1),
        actor_dps: dps(act.damage, dur_s),
        actor_damage: act.damage,
        actor_condi_dps: dps(act.condi, dur_s),
        actor_condi_damage: act.condi,
        actor_power_dps: dps(act.power, dur_s),
        actor_power_damage: act.power,
        actor_breakbar_damage: round(bb_actor, 1),
    }
}

// ===== JsonGameplayStats/JsonGameplayStatsAll(OffensiveStatistics.cs)=====

pub struct OffensiveAcc {
    pub total_count: i64,
    pub total_dmg: i64,
    pub direct_count: i64,
    pub direct_dmg: i64,
    pub damage_count: i64,
    pub damage: i64,
    pub direct_hit_count: i64,
    pub direct_hit_dmg: i64,
    pub power_count: i64,
    pub power_dmg: i64,
    pub power_90_count: i64,
    pub power_90_dmg: i64,
    pub leech_count: i64,
    pub leech_dmg: i64,
    pub condi_count: i64,
    pub condi_dmg: i64,
    pub condi_90_count: i64,
    pub condi_90_dmg: i64,
    pub critable_direct: i64,
    pub crit_count: i64,
    pub crit_dmg: i64,
    pub flank: i64,
    pub glance: i64,
    pub moving: i64,
    pub missed: i64,
    pub blocked: i64,
    pub evaded: i64,
    pub interrupts: i64,
    pub invulned: i64,
    pub killed: i64,
    pub downed: i64,
    pub against_down_count: i64,
    pub against_down_dmg: i64,
    pub down_contribution: i64,
    pub down_contribution_per_skill: std::collections::BTreeMap<i64, i64>,
    pub cc_down_count: i64,
    pub cc_down_dur: f64,
    pub cc_count: i64,
    pub cc_dur: f64,
}

impl Default for OffensiveAcc {
    fn default() -> Self {
        OffensiveAcc {
            total_count: 0, total_dmg: 0, direct_count: 0, direct_dmg: 0, damage_count: 0,
            damage: 0, direct_hit_count: 0, direct_hit_dmg: 0, power_count: 0, power_dmg: 0,
            power_90_count: 0, power_90_dmg: 0, leech_count: 0, leech_dmg: 0, condi_count: 0,
            condi_dmg: 0, condi_90_count: 0, condi_90_dmg: 0, critable_direct: 0, crit_count: 0,
            crit_dmg: 0, flank: 0, glance: 0, moving: 0, missed: 0, blocked: 0, evaded: 0,
            interrupts: 0, invulned: 0, killed: 0, downed: 0, against_down_count: 0,
            against_down_dmg: 0, down_contribution: 0, down_contribution_per_skill:
                std::collections::BTreeMap::new(),
            cc_down_count: 0, cc_down_dur: 0.0, cc_count: 0, cc_dur: 0.0,
        }
    }
}

/// OffensiveStatistics 构造逻辑(逐行;注意 downed/killed/interrupts 计数
/// 挂在所有行,与 from==actor 无关 —— C# 原样)。
#[allow(clippy::too_many_arguments)]
pub fn compute_offensive(
    actor: &ActorRows,
    ctx: &Ctx,
    start: i64,
    end: i64,
    target: Option<AgentId>,
    can_crit: &dyn Fn(i64) -> bool,
) -> OffensiveAcc {
    let mut acc = OffensiveAcc::default();
    let log = ctx.log;
    let rows = actor.health_out(start, end, target);
    for hl in rows {
        let Some(row) = row_view(ctx, &hl) else { continue };
        let is_self = hl.line.from != NO_AGENT
            && log.agents.englobing_root(hl.line.from) == actor.root;
        if is_self {
            if !row.is_no_damage {
                acc.total_count += 1;
                acc.total_dmg += i64::from(row.health_damage);
                if !row.is_non_direct {
                    acc.direct_count += 1;
                    acc.direct_dmg += i64::from(row.health_damage);
                }
            }
            if row.has_hit {
                acc.damage_count += 1;
                acc.damage += i64::from(row.health_damage);
                let is_condi = row.condition_based == Some(true);
                if is_condi {
                    acc.condi_count += 1;
                    acc.condi_dmg += i64::from(row.health_damage);
                    if row.is_over_ninety {
                        acc.condi_90_count += 1;
                        acc.condi_90_dmg += i64::from(row.health_damage);
                    }
                } else {
                    if row.is_non_direct {
                        if row.is_life_leech {
                            acc.leech_count += 1;
                            acc.leech_dmg += i64::from(row.health_damage);
                        }
                    } else {
                        if can_crit(crate::signed_id(row.skill_id)) {
                            if row.has_crit {
                                acc.crit_count += 1;
                                acc.crit_dmg += i64::from(row.health_damage);
                            }
                            acc.critable_direct += 1;
                        }
                        acc.direct_hit_count += 1;
                        acc.direct_hit_dmg += i64::from(row.health_damage);
                        if row.is_flanking {
                            acc.flank += 1;
                        }
                        if row.has_glanced {
                            acc.glance += 1;
                        }
                    }
                    acc.power_count += 1;
                    acc.power_dmg += i64::from(row.health_damage);
                    if row.is_over_ninety {
                        acc.power_90_count += 1;
                        acc.power_90_dmg += i64::from(row.health_damage);
                    }
                }
                if row.against_downed {
                    acc.against_down_count += 1;
                    acc.against_down_dmg += i64::from(row.health_damage);
                }
                if row.against_moving {
                    acc.moving += 1;
                }
                // down contribution(现代路径;dl.To.IsDownedBeforeNext90,
                // 按行实际承伤者状态;伪 target 等无状态 agent 恒 false)
                if row.has_hit
                    && ctx
                        .downs
                        .get(hl.line.to)
                        .is_some_and(|dc| dc.is_down_before_next_90(hl.line.to, row.time))
                {
                    acc.down_contribution += i64::from(row.health_damage);
                    *acc
                        .down_contribution_per_skill
                        .entry(crate::signed_id(row.skill_id))
                        .or_default() += i64::from(row.health_damage);
                }
            }
            if row.is_absorbed {
                acc.invulned += 1;
            }
            if row.is_blind {
                acc.missed += 1;
            }
            if row.is_evaded {
                acc.evaded += 1;
            }
            if row.is_blocked {
                acc.blocked += 1;
            }
        }
        if row.has_interrupted {
            acc.interrupts += 1;
        }
        if row.has_killed {
            acc.killed += 1;
        }
        if row.has_downed {
            acc.downed += 1;
        }
    }
    // crowd control 系(GetOutgoingCrowdControlEvents;仅 from 行)
    let cc_rows = if let Some(t) = target {
        actor
            .cc_out(start, end)
            .into_iter()
            .filter(|l| {
                let Some(ta) = log.agents.slot(t) else { return false };
                let root = resolve_cc_from(log, l);
                if root == NO_AGENT {
                    return false;
                }
                let _ = root;
                log.agents.englobing_root(l.to) == log.agents.englobing_root(t)
                    && ta.first_aware <= l.time
                    && l.time <= ta.last_aware
            })
            .collect::<Vec<_>>()
    } else {
        actor.cc_out(start, end)
    };
    for l in cc_rows {
        let dur = actor.cc_duration(&l);
        acc.cc_count += 1;
        acc.cc_dur += dur as f64;
        if ctx
            .downs
            .get(l.to)
            .is_some_and(|dc| dc.is_down_before_next_90(l.to, l.time))
        {
            acc.cc_down_count += 1;
            acc.cc_down_dur += dur as f64;
        }
    }
    acc
}

fn resolve_cc_from(log: &ParsedLog, l: &DmgLine) -> AgentId {
    let mut t = log.agents.clone();
    let id = t.resolve_agent(
        match &log.events[l.ev] {
            CombatEvent::CrowdControl(f) => f.skill.from,
            _ => 0,
        },
        l.time,
    );
    if id != NO_AGENT { id } else { NO_AGENT }
}

pub struct DownCtx {
    downs: Vec<Segment>,
    health_points: Vec<(i64, f64)>,
    log_end: i64,
}

impl DownCtx {
    pub fn new(log: &ParsedLog, idx: &EventIndex, target: AgentId) -> Self {
        let Some(a) = log.agents.slot(target) else {
            return DownCtx { downs: vec![], health_points: vec![], log_end: log.log_data.log_end };
        };
        // down 段:全部状态事件(C# GetAgentStatus;Alive/Spawn 截断 down
        // 段 —— 只喂 down/dead 会让被救起的玩家段延伸、误判 is_downed)。
        let mut events: Vec<(i64, u8)> = Vec::new();
        for (map, kind) in [
            (&idx.down, 0u8),
            (&idx.dead, 1),
            (&idx.despawn, 2),
            (&idx.alive, 3),
            (&idx.spawn, 3),
        ] {
            if let Some(list) = map.get(&target) {
                for &e in list {
                    events.push((log.events[e].time().unwrap_or(0), kind));
                }
            }
        }
        events.sort_by_key(|x| x.0);
        let (_, downs, _, _) = gw2ei_model::status_segments(&events, a.first_aware, a.last_aware);
        // health points:GetHealthUpdates 的 list_from_states
        let hp: Vec<(i64, f64)> = health_points_of(log, idx, target);
        DownCtx { downs: downs.to_vec(), health_points: hp, log_end: log.log_data.log_end }
    }
    /// `IsDowned(log, curTime)`:当前处于 down 段内(段含端点;C# Segment
    /// Contains)。DownCtx.downs 只有 down 段(down+dead 事件合成)。
    pub fn is_downed(&self, time: i64) -> bool {
        self.downs
            .iter()
            .any(|s| s.start <= time && time <= s.end)
    }
    /// `IsDownedBeforeNext90`(SingleActorStatusHelper.cs:404-438)。
    pub fn is_down_before_next_90(&self, _to: AgentId, time: i64) -> bool {
        let hp_at = |t: i64| -> f64 {
            self.health_points
                .iter()
                .filter(|&&(st, _)| st <= t)
                .map(|&(_, v)| v)
                .next_back()
                .unwrap_or(-1.0)
        };
        if self.is_downed(time) {
            return false;
        }
        if hp_at(time) > 90.0 {
            return false;
        }
        let next_down = self
            .downs
            .iter()
            .find(|d| d.intersecting_area(time, self.log_end) > 0.0);
        let Some(next_down) = next_down else { return false };
        let next_90 = self
            .health_points
            .iter()
            .filter(|&&(st, v)| st > time && v > 90.0)
            .map(|&(st, _)| st)
            .next();
        match next_90 {
            None => true,
            Some(t) => t >= next_down.start,
        }
    }
}

/// per-承伤者状态表(OffensiveStats/dists 每行按 `to` 判定):只对有
/// down/dead/health 事件的 agent 构建;查询缺省 = 无状态 → 判定恒 false
/// (与 C# 空段结果一致)。
pub struct DownMap {
    map: std::collections::HashMap<AgentId, DownCtx>,
}

impl DownMap {
    pub fn new(log: &ParsedLog, idx: &EventIndex) -> Self {
        let mut ids: std::collections::BTreeSet<AgentId> = Default::default();
        for m in [&idx.down, &idx.dead, &idx.health_update] {
            ids.extend(m.keys().copied());
        }
        let map = ids
            .into_iter()
            .map(|id| (id, DownCtx::new(log, idx, id)))
            .collect();
        DownMap { map }
    }
    pub fn get(&self, agent: AgentId) -> Option<&DownCtx> {
        self.map.get(&agent)
    }
}

fn health_points_of(log: &ParsedLog, idx: &EventIndex, agent: AgentId) -> Vec<(i64, f64)> {
    let mut states: Vec<(i64, f64)> = Vec::new();
    if let Some(rows) = idx.health_update.get(&agent) {
        for &e in rows {
            if let CombatEvent::HealthUpdate(f) = &log.events[e] {
                states.push((f.base.time, f.percent));
            }
        }
    }
    gw2ei_model::percent_points(&states, log.log_data.log_start, log.log_data.log_end)
}

/// EventIndex 重建(旧调用方;每次调用 O(events))。
fn build_event_index_once(log: &ParsedLog) -> EventIndex {
    gw2ei_model::build_event_index(log)
}

/// JsonGameplayStats 填装(38 键)。
pub fn game_stats(acc: &OffensiveAcc) -> JsonGameplayStats {
    JsonGameplayStats {
        total_damage_count: acc.total_count,
        total_dmg: acc.total_dmg,
        direct_damage_count: acc.direct_count,
        direct_dmg: acc.direct_dmg,
        connected_damage_count: acc.damage_count,
        connected_dmg: acc.damage,
        connected_direct_damage_count: acc.direct_hit_count,
        connected_direct_dmg: acc.direct_hit_dmg,
        connected_power_count: acc.power_count,
        connected_power_damage: acc.power_dmg,
        connected_power_above_90_hp_count: acc.power_90_count,
        connected_power_above_90_hp_damage: acc.power_90_dmg,
        connected_life_leech_count: acc.leech_count,
        connected_life_leech_damage: acc.leech_dmg,
        connected_condition_count: acc.condi_count,
        connected_condition_damage: acc.condi_dmg,
        connected_condition_above_90_hp_count: acc.condi_90_count,
        connected_condition_above_90_hp_damage: acc.condi_90_dmg,
        critable_direct_damage_count: acc.critable_direct,
        critical_rate: acc.crit_count,
        critical_dmg: acc.crit_dmg,
        flanking_rate: acc.flank,
        against_moving_rate: acc.moving,
        glance_rate: acc.glance,
        missed: acc.missed,
        evaded: acc.evaded,
        blocked: acc.blocked,
        interrupts: acc.interrupts,
        invulned: acc.invulned,
        killed: acc.killed,
        downed: acc.downed,
        against_downed_count: acc.against_down_count,
        against_downed_damage: acc.against_down_dmg,
        down_contribution: acc.down_contribution,
        applied_crowd_control_down_contribution: acc.cc_down_count,
        applied_crowd_control_duration_down_contribution: acc.cc_down_dur,
        applied_crowd_control: acc.cc_count,
        applied_crowd_control_duration: acc.cc_dur,
    }
}

// ===== GameplayStatistics(cast 分析)=====

pub struct GameplayAcc {
    pub interrupted_count: i64,
    pub interrupted_duration: f64,
    pub after_cast_interrupted_count: i64,
    pub after_cast_interrupted_duration: f64,
    pub swap_count: i64,
    pub cast_time: i64,
    pub cast_uptime: f64,
    pub cast_uptime_no_aa: f64,
}

/// 玩家/非玩家的 gameplay(不含 fake)。`is_aa(id)` 由调用方注入。
pub fn compute_gameplay(
    log: &ParsedLog,
    actor: AgentId,
    start: i64,
    end: i64,
    cast_rows: &[usize],
    is_aa: &dyn Fn(i64) -> bool,
) -> GameplayAcc {
    let mut acc = GameplayAcc {
        interrupted_count: 0,
        interrupted_duration: 0.0,
        after_cast_interrupted_count: 0,
        after_cast_interrupted_duration: 0.0,
        swap_count: 0,
        cast_time: 0,
        cast_uptime: 0.0,
        cast_uptime_no_aa: 0.0,
    };
    use gw2ei_model::AnimationStatus;
    // 全 cast(含武器切换;InitCastEvents 语义:animated + swap 合成)
    let mut casts: Vec<usize> = cast_rows.to_vec();
    casts.sort_by_key(|&i| (log.events[i].time().unwrap_or(0), is_swap_event(log, i)));
    for &i in &casts {
        let evt = &log.events[i];
        let (status, saved, _actual, skill_id) = match evt {
            CombatEvent::AnimatedCast(f) => (f.status, f.saved_duration, f.actual_duration, f.skill_id),
            CombatEvent::Emote(f) => (f.base.status, f.base.saved_duration, f.base.actual_duration, f.base.skill_id),
            CombatEvent::GadgetInteract(f) => (f.status, f.saved_duration, f.actual_duration, f.skill_id),
            CombatEvent::BundlePickUp(f) => (f.base.status, f.base.saved_duration, f.base.actual_duration, f.base.skill_id),
            CombatEvent::WeaponSwap(_) => {
                // WeaponSwap 行参与 cast 流:ActualDuration=0、不打断;C#
                // WeaponSwapEvent.SkillID == WeaponSwap(-2) → IsSwap 恒真。
                acc.swap_count += 1;
                continue;
            }
            _ => continue,
        };
        match status {
            gw2ei_model::AnimationStatus::Interrupted => {
                acc.interrupted_count += 1;
                acc.interrupted_duration += f64::from(saved);
            }
            gw2ei_model::AnimationStatus::Reduced => {
                acc.after_cast_interrupted_count += 1;
                acc.after_cast_interrupted_duration += f64::from(saved);
            }
            _ => {}
        }
        if is_swap(crate::signed_id(skill_id)) {
            acc.swap_count += 1;
        }
    }
    acc.after_cast_interrupted_duration = round3(acc.after_cast_interrupted_duration);
    acc.interrupted_duration = -round3(acc.interrupted_duration);
    // uptime 用相交窗口(GetIntersectingCastEvents)
    let mut total = 0i64;
    for &i in &casts {
        let evt = &log.events[i];
        let (time, actual, status, skill_id) = match evt {
            CombatEvent::AnimatedCast(f) => (f.time, f.actual_duration, f.status, f.skill_id),
            CombatEvent::Emote(f) => (f.base.time, f.base.actual_duration, f.base.status, f.base.skill_id),
            CombatEvent::GadgetInteract(f) => (f.time, f.actual_duration, f.status, f.skill_id),
            CombatEvent::BundlePickUp(f) => (f.base.time, f.base.actual_duration, f.base.status, f.base.skill_id),
            CombatEvent::WeaponSwap(_) => continue,
            _ => continue,
        };
        let end_time = time + i64::from(actual);
        // KeepIntersectingCastLog(整段相交)
        let intersects = (time >= start && time <= end)
            || (end_time >= start && end_time <= end)
            || (time <= start && end_time >= end);
        if !intersects {
            continue;
        }
        let value = (end_time.min(end) - time.max(start)).max(0);
        acc.cast_time += value;
        if status == AnimationStatus::Interrupted || status == AnimationStatus::Unknown {
            continue;
        }
        acc.cast_uptime += value as f64;
        if !is_aa(crate::signed_id(skill_id)) {
            acc.cast_uptime_no_aa += value as f64;
        }
    }
    let _ = &mut total;
    let time_in_combat = actor_time_in_combat(log, actor, start, end).max(1);
    acc.cast_uptime /= time_in_combat as f64;
    acc.cast_uptime_no_aa /= time_in_combat as f64;
    acc.cast_uptime = round(acc.cast_uptime * 100.0, 3);
    acc.cast_uptime_no_aa = round(acc.cast_uptime_no_aa * 100.0, 3);
    acc
}

/// WeaponSwap 事件(排序时 swap 排同刻最后)。
fn is_swap_event(log: &ParsedLog, i: usize) -> bool {
    matches!(log.events[i], CombatEvent::WeaponSwap(_))
}

/// GetTimeSpentInCombat(SingleActorStatusHelper.cs:356-388;非 englobing)。
pub fn actor_time_in_combat(log: &ParsedLog, actor: AgentId, start: i64, end: i64) -> i64 {
    let idx = build_event_index_once(log);
    let mut enters: Vec<(i64, bool)> = Vec::new();
    if let Some(rows) = idx.enter_combat.get(&actor) {
        for &e in rows {
            enters.push((log.events[e].time().unwrap_or(0), true));
        }
    }
    if let Some(rows) = idx.exit_combat.get(&actor) {
        for &e in rows {
            enters.push((log.events[e].time().unwrap_or(0), false));
        }
    }
    enters.sort_by_key(|x| x.0);
    let mut time = 0i64;
    let mut in_combat = false;
    let mut last_enter = 0i64;
    let n = enters.len();
    for (t, is_in) in &enters {
        if *is_in {
            in_combat = true;
            last_enter = *t;
        } else if in_combat {
            in_combat = false;
            time += (t.min(&end) - last_enter.max(start)).max(0);
        }
    }
    if time == 0 && n == 0 {
        return end - start;
    }
    if time == 0 {
        if let Some((t, false)) = enters.first().copied().filter(|x| !x.1) {
            time += (t.min(end) - start).max(0);
        } else {
            time = (end - start).max(0);
        }
    }
    time
}

// ===== Defense(DefensePerTargetStatistics + DefenseAllStatistics)=====

pub struct DefenseAcc {
    pub damage_taken: i64,
    pub damage_taken_count: i64,
    pub damage_barrier: i64,
    pub damage_barrier_count: i64,
    pub condi_taken: i64,
    pub condi_taken_count: i64,
    pub power_taken: i64,
    pub power_taken_count: i64,
    pub leech_taken: i64,
    pub leech_taken_count: i64,
    pub strike_taken: i64,
    pub strike_taken_count: i64,
    pub downed_taken: i64,
    pub downed_taken_count: i64,
    pub bb_taken: f64,
    pub bb_taken_count: i64,
    pub blocked: i64,
    pub missed: i64,
    pub evaded: i64,
    pub invulned: i64,
    pub interrupted: i64,
    pub cc_received: i64,
    pub cc_received_duration: f64,
    // All 面
    pub down_count: i64,
    pub down_duration: i64,
    pub dead_count: i64,
    pub dead_duration: i64,
    pub dc_count: i64,
    pub dc_duration: i64,
    pub stun_break: i64,
    pub removed_stun_duration: f64,
    // strip(分类统计在 support/defense 输出中)
    pub boon_strips: i64,
    pub boon_strips_time: f64,
    pub condition_cleanses: i64,
    pub condition_cleanses_time: f64,
    pub dodge_count: i64,
}

impl Default for DefenseAcc {
    fn default() -> Self {
        DefenseAcc {
            damage_taken: 0, damage_taken_count: 0, damage_barrier: 0, damage_barrier_count: 0,
            condi_taken: 0, condi_taken_count: 0, power_taken: 0, power_taken_count: 0,
            leech_taken: 0, leech_taken_count: 0, strike_taken: 0, strike_taken_count: 0,
            downed_taken: 0, downed_taken_count: 0, bb_taken: 0.0, bb_taken_count: 0,
            blocked: 0, missed: 0, evaded: 0, invulned: 0, interrupted: 0, cc_received: 0,
            cc_received_duration: 0.0, down_count: 0, down_duration: 0, dead_count: 0,
            dead_duration: 0, dc_count: 0, dc_duration: 0, stun_break: 0,
            removed_stun_duration: 0.0, boon_strips: 0, boon_strips_time: 0.0,
            condition_cleanses: 0, condition_cleanses_time: 0.0, dodge_count: 0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute_defense(
    actor: &ActorRows,
    ctx: &Ctx,
    start: i64,
    end: i64,
    is_player: bool,
    is_ele: bool,
    buff_apply_downed: i64,
    vapor_removes: &[i64],
    strip_counts: (i64, f64, i64, f64),
) -> DefenseAcc {
    let log = ctx.log;
    let mut a = DefenseAcc::default();
    for hl in actor.health_in(start, end, None) {
        let Some(row) = row_view(ctx, &hl) else { continue };
        if row.has_hit {
            a.damage_taken += i64::from(row.health_damage);
            a.damage_taken_count += 1;
            if row.condition_based == Some(true) {
                a.condi_taken += i64::from(row.health_damage);
                a.condi_taken_count += 1;
            } else {
                if row.is_non_direct {
                    if row.is_life_leech {
                        a.leech_taken += i64::from(row.health_damage);
                        a.leech_taken_count += 1;
                    }
                } else {
                    a.strike_taken += i64::from(row.health_damage);
                    a.strike_taken_count += 1;
                }
                a.power_taken += i64::from(row.health_damage);
                a.power_taken_count += 1;
            }
            if row.shield_damage > 0 {
                a.damage_barrier += i64::from(row.shield_damage);
                a.damage_barrier_count += 1;
            }
            if row.against_downed {
                a.downed_taken += i64::from(row.health_damage);
                a.downed_taken_count += 1;
            }
        }
        if row.is_blocked {
            a.blocked += 1;
        }
        if row.is_blind {
            a.missed += 1;
        }
        if row.is_absorbed {
            a.invulned += 1;
        }
        if row.is_evaded {
            a.evaded += 1;
        }
        if row.has_interrupted {
            a.interrupted += 1;
        }
    }
    for l in actor.cc_in(start, end) {
        a.cc_received += 1;
        a.cc_received_duration += actor.cc_duration(&l) as f64;
    }
    for l in actor.breakbar_in(start, end) {
        a.bb_taken += actor.breakbar_value(&l);
        a.bb_taken_count += 1;
    }
    a.bb_taken = round(a.bb_taken, 1);
    a.boon_strips = strip_counts.0;
    a.boon_strips_time = strip_counts.1;
    a.condition_cleanses = strip_counts.2;
    a.condition_cleanses_time = strip_counts.3;
    // ---- All 面 ----
    if is_player {
        a.down_count = buff_apply_downed;
        // Elementalist:2s 内有 VaporForm remove 的 apply 剔除
        if is_ele {
            a.down_count = a.down_count.saturating_sub(vapor_removes.len() as i64);
        }
    } else {
        a.down_count = 0;
        let idx = build_event_index_once(log);
        if let Some(rows) = idx.down.get(&actor.root) {
            a.down_count = rows
                .iter()
                .filter(|&&e| {
                    let t = log.events[e].time().unwrap_or(0);
                    start <= t && t <= end
                })
                .count() as i64;
        }
    }
    {
        let idx = build_event_index_once(log);
        a.dead_count = count_in(idx.dead.get(&actor.root), log, start, end);
        a.dc_count = count_in(idx.despawn.get(&actor.root), log, start, end);
        let mut events: Vec<(i64, u8)> = Vec::new();
        if let Some(rows) = idx.down.get(&actor.root) {
            for &e in rows {
                events.push((log.events[e].time().unwrap_or(0), 0));
            }
        }
        if let Some(rows) = idx.dead.get(&actor.root) {
            for &e in rows {
                events.push((log.events[e].time().unwrap_or(0), 1));
            }
        }
        if let Some(rows) = idx.despawn.get(&actor.root) {
            for &e in rows {
                events.push((log.events[e].time().unwrap_or(0), 2));
            }
        }
        if let Some(rows) = idx.alive.get(&actor.root) {
            for &e in rows {
                events.push((log.events[e].time().unwrap_or(0), 3));
            }
        }
        if let Some(rows) = idx.spawn.get(&actor.root) {
            for &e in rows {
                events.push((log.events[e].time().unwrap_or(0), 3));
            }
        }
        events.sort_by_key(|x| x.0);
        let (dead, down, dc, _) = {
            let a0 = log.agents.slot(actor.root).expect("slot");
            gw2ei_model::status_segments(&events, a0.first_aware, a0.last_aware)
        };
        a.down_duration = dead_area(&down, start, end);
        a.dead_duration = dead_area(&dead, start, end);
        a.dc_duration = dead_area(&dc, start, end);
        // StunBreak received
        if let Some(rows) = idx.stun_break_to.get(&actor.root) {
            for &e in rows {
                let t = log.events[e].time().unwrap_or(0);
                if start <= t && t <= end {
                    a.stun_break += 1;
                    if let CombatEvent::StunBreak(f) = &log.events[e] {
                        a.removed_stun_duration += f64::from(f.remaining_duration);
                    }
                }
            }
        }
        a.removed_stun_duration = round3(a.removed_stun_duration);
    }
    // Dodge 计数:GetCastEvents 中 IsDodge(技能 == DodgeID)
    {
        let idx = build_event_index_once(log);
        let mut dodge = 0;
        if let Some(rows) = idx.cast_by_caster.get(&actor.root) {
            for &e in rows {
                let t = log.events[e].time().unwrap_or(0);
                if start <= t && t <= end
                    && matches!(&log.events[e], CombatEvent::AnimatedCast(f) if crate::signed_id(f.skill_id) == DODGE_ID)
                {
                    dodge += 1;
                }
            }
        }
        a.dodge_count = dodge;
    }
    a
}

/// ArcDPSDodge20220307(SkillIDs.cs;evtc build ≥ 20220304 起,C# SkillData
/// DodgeID 取自 `GetArcDPSCustomIDs`)。
const DODGE_ID: i64 = 23275;

fn count_in(rows: Option<&Vec<usize>>, log: &ParsedLog, start: i64, end: i64) -> i64 {
    rows.map(|l| {
        l.iter()
            .filter(|&&e| {
                let t = log.events[e].time().unwrap_or(0);
                start <= t && t <= end
            })
            .count() as i64
    })
    .unwrap_or(0)
}

fn dead_area(segs: &[Segment], start: i64, end: i64) -> i64 {
    segs.iter().map(|s| s.intersecting_area(start, end) as i64).sum()
}

// ===== Support =====
pub struct SupportAcc {
    pub resurrect_count: i64,
    pub resurrect_time: f64,
    pub condi_cleanse: i64,
    pub condi_cleanse_time: f64,
    pub condi_cleanse_self: i64,
    pub condi_cleanse_time_self: f64,
    pub boon_strips: i64,
    pub boon_strips_time: f64,
    pub boon_strip_down: i64,
    pub boon_strip_down_time: f64,
    pub stun_break: i64,
    pub removed_stun_duration: f64,
    pub stun_break_self: i64,
    pub removed_stun_self_duration: f64,
}

impl Default for SupportAcc {
    fn default() -> Self {
        SupportAcc {
            resurrect_count: 0, resurrect_time: 0.0, condi_cleanse: 0, condi_cleanse_time: 0.0,
            condi_cleanse_self: 0, condi_cleanse_time_self: 0.0, boon_strips: 0,
            boon_strips_time: 0.0, boon_strip_down: 0, boon_strip_down_time: 0.0,
            stun_break: 0, removed_stun_duration: 0.0, stun_break_self: 0,
            removed_stun_self_duration: 0.0,
        }
    }
}
