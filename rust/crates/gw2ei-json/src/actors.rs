//! JsonPlayer/JsonNpc 构建(JsonPlayerBuilder/JsonNPCBuilder/JsonActorBuilder)。
//! 每 actor 统计按 phase 循环;P2 单 phase。

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, NO_AGENT, ParsedLog};

use crate::ctx::Ctx;
use crate::dto_b::*;
use crate::rows::{ActorRows, HealthLine};
use crate::skills::{SkillTable, WEAPON_SWAP_ID, weapon_set_ids};
use crate::stats;

const SERVER_DELAY: i64 = 10; // ParserHelper.ServerDelayConstant

#[derive(Default)]
pub struct Collectors {
    pub skill_map: std::collections::BTreeMap<i64, ()>,
    pub buff_map: std::collections::BTreeMap<i64, ()>,
    pub team_map: std::collections::BTreeSet<i64>,
    pub personal_buffs: std::collections::BTreeMap<String, std::collections::BTreeSet<i64>>,
}



// ===== 身份 =====

pub struct Identity {
    pub name: String,
    pub account: String,
    pub group: i64,
    pub spec_name: String,
    pub not_in_squad: bool,
    pub friendly_npc: bool,
    pub squadless: bool,
    pub is_englobed: bool,
}

/// 玩家/非小队玩家的共同身份 + 公共 actor 字段填装。
pub struct ActorBuilder<'a, 'c> {
    pub ctx: &'c Ctx<'a>,
    pub actor: ActorRows<'a>,
}

impl<'a, 'c> ActorBuilder<'a, 'c> {
    pub fn team_id(&self, agent: AgentId) -> i64 {
        let Some(a) = self.ctx.log.agents.slot(agent) else { return 0 };
        let addr = a.address;
        let mut team = 0i64;
        for &(src, time, into, from) in &self.ctx.meta.team_changes {
            if src == addr && time <= a.last_aware {
                team = if into > 0 { into } else if from > 0 { from } else { team };
            }
        }
        team
    }

    pub fn guild_id(&self, agent: AgentId) -> Option<String> {
        let addr = self.ctx.log.agents.slot(agent)?.address;
        let hex = self
            .ctx
            .meta
            .guild_events
            .iter()
            .find(|(src, _)| *src == addr)
            .map(|(_, h)| h.clone())?;
        if hex.len() == 32 {
            Some(format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            ))
        } else {
            Some(hex)
        }
    }

    /// GetHealth:MaxHealthUpdate 首见路径(C# SingleActor.GetHealth)。
    pub fn total_health(&self, agent: AgentId) -> i64 {
        let idx = &self.ctx.idx;
        let Some(a) = self.ctx.log.agents.slot(agent) else { return -1 };
        let mut updates: Vec<(i64, i32)> = Vec::new();
        if let Some(rows) = idx.max_health_update.get(&agent) {
            for &e in rows {
                if let CombatEvent::MaxHealthUpdate(f) = &self.ctx.log.events[e] {
                    updates.push((f.base.time, f.max_health));
                }
            }
        }
        if updates.is_empty() {
            return -1;
        }
        let time_check = {
            // lastDamage(>0 伤害)time - 5000 与 halfAware 取大
            let mut tc = a.half_aware();
            if let Some(rows) = idx.damage_to.get(&agent) {
                for &e in rows.iter().rev() {
                    let evt = &self.ctx.log.events[e];
                    if let CombatEvent::DirectHealthDamage(f)
                    | CombatEvent::NonDirectHealthDamage(f)
                    | CombatEvent::NoDamageHealthDamage(f) = evt
                        && f.health_damage > 0
                    {
                        tc = tc.max(a.first_aware.max(f.skill.time - 5000));
                        break;
                    }
                }
            }
            tc
        };
        let cand: Vec<(i64, i32)> = updates
            .iter()
            .filter(|(t, _)| *t < time_check + SERVER_DELAY)
            .copied()
            .collect();
        match cand.last() {
            Some(&(_, hp)) => i64::from(hp),
            None => i64::from(updates.iter().map(|(_, hp)| *hp).max().unwrap_or(0)),
        }
    }

    /// 状态段(dead/down/dc/actives;含 EventIndex 合成)。
    pub fn status(&self, agent: AgentId) -> (Vec<gw2ei_model::Segment>, Vec<gw2ei_model::Segment>, Vec<gw2ei_model::Segment>, Vec<gw2ei_model::Segment>) {
        let idx = &self.ctx.idx;
        let log = self.ctx.log;
        let Some(a) = log.agents.slot(agent) else {
            return (vec![], vec![], vec![], vec![]);
        };
        let mut events: Vec<(i64, u8)> = Vec::new();
        for (map, kind) in [
            (&idx.down, 0u8),
            (&idx.dead, 1),
            (&idx.despawn, 2),
            (&idx.alive, 3),
            (&idx.spawn, 3),
        ] {
            if let Some(rows) = map.get(&agent) {
                for &e in rows {
                    events.push((log.events[e].time().unwrap_or(0), kind));
                }
            }
        }
        events.sort_by_key(|x| x.0);
        gw2ei_model::status_segments(&events, a.first_aware, a.last_aware)
    }

    /// 1s 图(C# GetDamageGraph 系;行需 HasHit + 类型过滤)。
    pub fn dmg_graph_1s(
        &self,
        start: i64,
        end: i64,
        target: Option<AgentId>,
        ty: DmgType,
    ) -> Vec<i64> {
        let rows: Vec<(i64, i64)> = self
            .actor
            .health_out(start, end, target)
            .into_iter()
            .filter(|hl| {
                let Some(row) = stats::row_view(self.ctx, hl) else { return false };
                row.has_hit
                    && match ty {
                        DmgType::All => true,
                        DmgType::Power => row.condition_based != Some(true),
                        DmgType::Condition => row.condition_based == Some(true),
                    }
            })
            .map(|hl| {
                let row = stats::row_view(self.ctx, &hl).expect("row");
                (hl.line.time, i64::from(row.health_damage))
            })
            .collect();
        gw2ei_model::damage_graph_1s(&rows, start, end)
    }

    /// 承伤 1s 图。
    pub fn dmg_taken_graph_1s(&self, start: i64, end: i64, ty: DmgType) -> Vec<i64> {
        let rows: Vec<(i64, i64)> = self
            .actor
            .health_in(start, end, None)
            .into_iter()
            .filter(|hl| {
                let Some(row) = stats::row_view(self.ctx, hl) else { return false };
                row.has_hit
                    && match ty {
                        DmgType::All => true,
                        DmgType::Power => row.condition_based != Some(true),
                        DmgType::Condition => row.condition_based == Some(true),
                    }
            })
            .map(|hl| {
                let row = stats::row_view(self.ctx, &hl).expect("row");
                (hl.line.time, i64::from(row.health_damage))
            })
            .collect();
        gw2ei_model::damage_graph_1s(&rows, start, end)
    }

    /// 健康/屏障百分比序列([start, value] 双元素数组;JsonActorBuilder:100-101)。
    pub fn percent_series(&self, agent: AgentId, kind: PercentKind) -> Vec<Vec<f64>> {
        let idx = &self.ctx.idx;
        let log = self.ctx.log;
        let map = match kind {
            PercentKind::Health => &idx.health_update,
            PercentKind::Barrier => &idx.barrier_update,
        };
        let mut states: Vec<(i64, f64)> = Vec::new();
        if let Some(rows) = map.get(&agent) {
            for &e in rows {
                let t = log.events[e].time().unwrap_or(0);
                let p = match (&log.events[e], kind) {
                    (CombatEvent::HealthUpdate(f), PercentKind::Health)
                    | (CombatEvent::BarrierUpdate(f), PercentKind::Barrier) => f.percent,
                    _ => 0.0,
                };
                states.push((t, p));
            }
        }
        states.sort_by_key(|x| x.0);
        gw2ei_model::percent_points(&states, log.log_data.log_start, log.log_data.log_end)
            .into_iter()
            .map(|(t, v)| vec![t as f64, v])
            .collect()
    }

    /// 状态段区域与活跃时长。
    pub fn active_duration(&self, agent: AgentId, start: i64, end: i64) -> i64 {
        let (dead, _down, dc, _act) = self.status(agent);
        let total = end - start;
        let dead_a: i64 = dead.iter().map(|s| s.intersecting_area(start, end) as i64).sum();
        let dc_a: i64 = dc.iter().map(|s| s.intersecting_area(start, end) as i64).sum();
        (total - dead_a - dc_a).max(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DmgType {
    All,
    Power,
    Condition,
}

#[derive(Clone, Copy)]
pub enum PercentKind {
    Health,
    Barrier,
}

// ===== damage dist(JsonDamageDistBuilder)=====

pub struct DistCtx<'a, 'c> {
    pub b: &'c ActorBuilder<'a, 'c>,
    pub skills: &'c SkillTable,
}

/// 按 skill id 分组(C# dlsByID = GroupBy(x.SkillID).ToDictionary;Rust 保首见
/// 序 — 用 Vec<(id, rows)> 并做稳定序;BTreeMap 会按键排序,与 C# 字典序
/// 不一定一致 —— 对拍按集合等价匹配,输出用首见序)。
pub struct DistGroup {
    pub id: i64,
    pub rows: Vec<HealthLine>,
}

/// dist 三件套构建(造成伤害侧;target 可空)。
pub fn build_damage_dist(
    b: &ActorBuilder,
    start: i64,
    end: i64,
    target: Option<AgentId>,
    skills: &SkillTable,
    cols: &mut Collectors,
    can_crit: &dyn Fn(i64) -> bool,
) -> Vec<JsonDamageDist> {
    let log = b.ctx.log;
    let rows = b.actor.health_out_just(start, end, target);
    let mut groups: Vec<(i64, Vec<HealthLine>)> = Vec::new();
    for hl in rows {
        let id = crate::signed_id(skill_id_of(log, &hl));
        match groups.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(hl),
            None => groups.push((id, vec![hl])),
        }
    }
    // breakbar 行按 skill 分组
    let mut bb_groups: Vec<(i64, Vec<crate::rows::DmgLine>)> = Vec::new();
    for l in b.actor.breakbar_out_just(start, end) {
        let id = crate::signed_id(dmg_line_skill_id(log, &l));
        match bb_groups.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(l),
            None => bb_groups.push((id, vec![l])),
        }
    }
    // down contribution per skill(现代;与 offensive 同步)
    let dc_map = down_contribution_map(b, start, end, target);
    let mut out: Vec<JsonDamageDist> = Vec::new();
    for (id, rows) in &groups {
        let indirect = rows.iter().any(|hl| {
            stats::row_view(b.ctx, hl)
                .map(|r| r.is_non_direct)
                .unwrap_or(false)
        });
        let brls: Vec<crate::rows::DmgLine> = bb_groups
            .iter()
            .find(|(g, _)| g == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        out.push(one_dist(
            b.ctx, skills, cols, *id, rows, &brls, indirect, &dc_map,
        ));
    }
    for (id, brls) in &bb_groups {
        if groups.iter().any(|(g, _)| g == id) {
            continue;
        }
        // 纯 breakbar 组
        if !skills.contains(*id) {
            // 占位技能名(collect 收口)
        }
        let mut d = empty_dist(*id);
        for l in brls {
            d.hits += 1;
            d.connected_hits += 1;
            d.total_breakbar_damage += b.actor.breakbar_value(l);
        }
        d.total_breakbar_damage = (d.total_breakbar_damage * 10.0).round_ties_even() / 10.0;
        skills_insert(skills, cols, *id);
        out.push(d);
    }
    let _ = can_crit;
    out
}

#[allow(clippy::too_many_arguments)]
fn one_dist(
    ctx: &Ctx,
    skills: &SkillTable,
    cols: &mut Collectors,
    id: i64,
    rows: &[HealthLine],
    brls: &[crate::rows::DmgLine],
    indirect: bool,
    dc_map: &std::collections::BTreeMap<i64, i64>,
) -> JsonDamageDist {
    let mut d = JsonDamageDist {
        total_damage: 0,
        total_breakbar_damage: 0.0,
        min: 0,
        max: 0,
        hits: 0,
        connected_hits: 0,
        crit: 0,
        glance: 0,
        flank: 0,
        against_moving: 0,
        missed: 0,
        invulned: 0,
        interrupted: 0,
        evaded: 0,
        blocked: 0,
        shield_damage: 0,
        crit_damage: 0,
        down_contribution: dc_map.get(&id).copied(),
        id,
        indirect_damage: indirect,
    };
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    for hl in rows {
        let Some(row) = stats::row_view(ctx, hl) else { continue };
        if !row.is_no_damage {
            d.hits += 1;
        }
        d.total_damage += i64::from(row.health_damage);
        if row.has_hit {
            min = min.min(i64::from(row.health_damage));
            max = max.max(i64::from(row.health_damage));
            if row.against_moving {
                d.against_moving += 1;
            }
        }
        if !indirect {
            if row.has_hit {
                if row.is_flanking {
                    d.flank += 1;
                }
                if row.has_glanced {
                    d.glance += 1;
                }
                if row.has_crit {
                    d.crit += 1;
                    d.crit_damage += i64::from(row.health_damage);
                }
            }
            if row.is_blind {
                d.missed += 1;
            }
            if row.is_evaded {
                d.evaded += 1;
            }
            if row.is_blocked {
                d.blocked += 1;
            }
            if row.has_interrupted {
                d.interrupted += 1;
            }
        }
        if row.has_hit {
            d.connected_hits += 1;
        }
        if row.is_absorbed {
            d.invulned += 1;
        }
        d.shield_damage += i64::from(row.shield_damage);
    }
    d.min = if min == i64::MAX { 0 } else { min };
    d.max = if max == i64::MIN { 0 } else { max };
    for l in brls {
        d.total_breakbar_damage += match &ctx.log.events[l.ev] { CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => f.value, _ => 0.0 };
    }
    d.total_breakbar_damage = (d.total_breakbar_damage * 10.0).round_ties_even() / 10.0;
    if indirect {
        buffs_insert(ctx, cols, id);
    } else {
        skills_insert(skills, cols, id);
    }
    d
}

fn empty_dist(id: i64) -> JsonDamageDist {
    JsonDamageDist {
        total_damage: 0,
        total_breakbar_damage: 0.0,
        min: 0,
        max: 0,
        hits: 0,
        connected_hits: 0,
        crit: 0,
        glance: 0,
        flank: 0,
        against_moving: 0,
        missed: 0,
        invulned: 0,
        interrupted: 0,
        evaded: 0,
        blocked: 0,
        shield_damage: 0,
        crit_damage: 0,
        down_contribution: None,
        id,
        indirect_damage: false,
    }
}

/// 承伤侧 dist(无 downContribution、无 indirect buff 合成语义差异:仍按
/// 直/间收集,但 downContribution 无)。
pub fn build_damage_taken_dist(
    b: &ActorBuilder,
    start: i64,
    end: i64,
    target: Option<AgentId>,
    skills: &SkillTable,
    cols: &mut Collectors,
) -> Vec<JsonDamageDist> {
    let log = b.ctx.log;
    let rows = b.actor.health_in(start, end, target);
    let mut groups: Vec<(i64, Vec<HealthLine>)> = Vec::new();
    for hl in rows {
        let id = crate::signed_id(skill_id_of(log, &hl));
        match groups.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(hl),
            None => groups.push((id, vec![hl])),
        }
    }
    let mut bb: Vec<(i64, Vec<crate::rows::DmgLine>)> = Vec::new();
    for l in b.actor.breakbar_in(start, end) {
        let id = crate::signed_id(dmg_line_skill_id(log, &l));
        match bb.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(l),
            None => bb.push((id, vec![l])),
        }
    }
    let mut out: Vec<JsonDamageDist> = Vec::new();
    for (id, rows) in &groups {
        let indirect = rows.iter().any(|hl| {
            stats::row_view(b.ctx, hl)
                .map(|r| r.is_non_direct)
                .unwrap_or(false)
        });
        let brls: Vec<crate::rows::DmgLine> = bb
            .iter()
            .find(|(g, _)| g == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        out.push(one_dist(b.ctx, skills, cols, *id, rows, &brls, indirect, &Default::default()));
    }
    for (id, brls) in &bb {
        if groups.iter().any(|(g, _)| g == id) {
            continue;
        }
        let mut d = empty_dist(*id);
        for l in brls {
            d.hits += 1;
            d.connected_hits += 1;
            d.total_breakbar_damage += b.actor.breakbar_value(l);
        }
        d.total_breakbar_damage = (d.total_breakbar_damage * 10.0).round_ties_even() / 10.0;
        skills_insert(skills, cols, *id);
        out.push(d);
    }
    out
}

fn skill_id_of(log: &ParsedLog, hl: &HealthLine) -> u32 {
    match &log.events[hl.line.ev] {
        CombatEvent::DirectHealthDamage(f)
        | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => f.skill.skill_id,
        _ => 0,
    }
}

fn dmg_line_skill_id(log: &ParsedLog, l: &crate::rows::DmgLine) -> u32 {
    match &log.events[l.ev] {
        CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => f.skill.skill_id,
        CombatEvent::CrowdControl(f) => f.skill.skill_id,
        _ => 0,
    }
}

fn down_contribution_map(
    b: &ActorBuilder,
    start: i64,
    end: i64,
    target: Option<AgentId>,
) -> std::collections::BTreeMap<i64, i64> {
    let mut map: std::collections::BTreeMap<i64, i64> = Default::default();
    for hl in b.actor.health_out_just(start, end, target) {
        let Some(row) = stats::row_view(b.ctx, &hl) else { continue };
        // per-row 承伤者状态(OffensiveStatistics.cs:91-100 现代路径);
        // 伪 target 等无状态 agent 恒 false
        if row.has_hit
            && b
                .ctx
                .downs
                .get(hl.line.to)
                .is_some_and(|dc| dc.is_down_before_next_90(hl.line.to, hl.line.time))
        {
            *map.entry(crate::signed_id(row.skill_id)).or_default() += i64::from(row.health_damage);
        }
    }
    map
}

pub fn skills_insert(skills: &SkillTable, cols: &mut Collectors, id: i64) {
    cols.skill_map.insert(id, ());
    let _ = skills;
}
pub fn buffs_insert(ctx: &Ctx, cols: &mut Collectors, id: i64) {
    cols.buff_map.insert(id, ());
    let _ = ctx;
}

// ===== rotation(JsonRotationBuilder;P1 cast + P3b instant 部分)=====

/// rotation 合并行:事件行(animated/emote/gadget/bundle/weapon swap)+
/// instant-cast 合成行(P3b)。C# `InitCastEvents`(SingleActor.cs:599-618):
/// animated + instant 先入,weapon swaps 随后;SortByTimeThenNegatedSwap
/// 稳定序(同刻 swap 靠后)。
#[derive(Debug, Clone, Copy)]
pub enum CastRef {
    Ev(usize),
    Instant { time: i64, skill_id: i64 },
}

impl CastRef {
    pub fn time(&self, log: &ParsedLog) -> Option<i64> {
        match self {
            CastRef::Ev(i) => log.events[*i].time(),
            CastRef::Instant { time, .. } => Some(*time),
        }
    }
    pub fn is_swap(&self, log: &ParsedLog) -> bool {
        matches!(self, CastRef::Ev(i) if matches!(&log.events[*i], CombatEvent::WeaponSwap(_)))
    }
    /// (time, actual_duration)。
    pub fn span(&self, log: &ParsedLog) -> (i64, i64) {
        match self {
            CastRef::Ev(i) => match &log.events[*i] {
                CombatEvent::AnimatedCast(f) => (f.time, i64::from(f.actual_duration)),
                CombatEvent::Emote(f) => (f.base.time, i64::from(f.base.actual_duration)),
                CombatEvent::GadgetInteract(f) => (f.time, i64::from(f.actual_duration)),
                CombatEvent::BundlePickUp(f) => (f.base.time, i64::from(f.base.actual_duration)),
                CombatEvent::WeaponSwap(f) => (f.time, 0),
                _ => (0, 0),
            },
            CastRef::Instant { time, .. } => (*time, 0),
        }
    }
    pub fn skill_id(&self, log: &ParsedLog) -> i64 {
        match self {
            CastRef::Ev(i) => crate::signed_id(match &log.events[*i] {
                CombatEvent::WeaponSwap(_) => WEAPON_SWAP_ID as u32,
                CombatEvent::AnimatedCast(f) => f.skill_id,
                CombatEvent::Emote(f) => f.base.skill_id,
                CombatEvent::GadgetInteract(f) => f.skill_id,
                CombatEvent::BundlePickUp(f) => f.base.skill_id,
                _ => 0,
            }),
            CastRef::Instant { skill_id, .. } => *skill_id,
        }
    }
}

/// GetIntersectingCastEvents + GroupBy(SkillID);每 cast 一行 JsonSkill。
pub fn build_rotation(
    b: &ActorBuilder,
    start: i64,
    end: i64,
    skills: &SkillTable,
    cols: &mut Collectors,
    cast_refs: &[CastRef],
) -> Option<Vec<JsonRotation>> {
    let log = b.ctx.log;
    let mut keep: Vec<CastRef> = Vec::new();
    for &cr in cast_refs {
        let (time, actual) = cr.span(log);
        let end_time = time + actual;
        let inside = (time >= start && time <= end)
            || (end_time >= start && end_time <= end)
            || (time <= start && end_time >= end);
        if inside {
            keep.push(cr);
        }
    }
    // cast 合成序:时间升序、同刻 swap 靠后(C# SortByTimeThenNegatedSwap)
    keep.sort_by_key(|cr| (cr.time(log).unwrap_or(0), cr.is_swap(log)));
    if keep.is_empty() {
        return None;
    }
    let mut groups: Vec<(i64, Vec<CastRef>)> = Vec::new();
    for &cr in &keep {
        let id = cr.skill_id(log);
        match groups.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(cr),
            None => groups.push((id, vec![cr])),
        }
    }
    let mut out: Vec<JsonRotation> = Vec::new();
    for (id, casts) in &groups {
        skills_insert(skills, cols, *id);
        let skills_json = casts
            .iter()
            .map(|&cr| {
                let (time, actual, saved, accel) = match cr {
                    CastRef::Ev(i) => {
                        let evt = &log.events[i];
                        match evt {
                            CombatEvent::AnimatedCast(f) => (
                                f.time,
                                i64::from(f.actual_duration),
                                i64::from(f.saved_duration),
                                f.acceleration,
                            ),
                            CombatEvent::Emote(f) => (
                                f.base.time,
                                i64::from(f.base.actual_duration),
                                i64::from(f.base.saved_duration),
                                f.base.acceleration,
                            ),
                            CombatEvent::GadgetInteract(f) => (
                                f.time,
                                i64::from(f.actual_duration),
                                i64::from(f.saved_duration),
                                f.acceleration,
                            ),
                            CombatEvent::BundlePickUp(f) => (
                                f.base.time,
                                i64::from(f.base.actual_duration),
                                i64::from(f.base.saved_duration),
                                f.base.acceleration,
                            ),
                            // WeaponSwapEvent(cast 流中的 duration 0)
                            CombatEvent::WeaponSwap(f) => (f.time, 0, 0, 0.0),
                            _ => (0, 0, 0, 0.0),
                        }
                    }
                    // InstantCastEvent:ActualDuration/Saved/Acceleration = 0
                    CastRef::Instant { time, .. } => (time, 0, 0, 0.0),
                };
                JsonSkill {
                    cast_time: time,
                    duration: actual,
                    time_gained: saved,
                    quickness: accel,
                    ignore_on_rotation_render: None,
                }
            })
            .collect();
        out.push(JsonRotation { id: *id, skills: skills_json });
    }
    Some(out)
}


/// 事件行是否为 WeaponSwap(CastRef 的 is_swap 等价;estimate_weapons 用)。
pub fn is_swap_kind(log: &ParsedLog, i: usize) -> bool {
    matches!(&log.events[i], CombatEvent::WeaponSwap(_))
}

// ===== weaponSets(EstimateWeapons)=====

#[derive(Clone)]
pub struct WeaponSetOut {
    pub weapons: Vec<String>,
    pub start: i64,
    pub end: i64,
}

/// GetWeaponSets:首套 [logStart, logEnd];仅 PlayerActor 且 computeCast。
pub fn estimate_weapons(
    log: &ParsedLog,
    _agent: AgentId,
    is_player_actor: bool,
    compute_cast: bool,
    cast_rows: &[usize],
    swap_rows: &[usize],
    skills: &SkillTable,
) -> Vec<WeaponSetOut> {
    let mut sets = vec![WeaponSetOut {
        weapons: vec![
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
            "Unknown".to_string(),
        ],
        start: log.log_data.log_start,
        end: log.log_data.log_end,
    }];
    if !is_player_actor || !compute_cast {
        return sets;
    }
    // 合成流:animation casts + weapon swaps。C# EstimateWeapons 跳过
    // `ActualDuration == 0` 的 cast(瞬发技能无动画帧不可推断武器;
    // WeaponSwap 例外 —— ActualDuration=0 但要处理)。
    let mut all: Vec<usize> = cast_rows
        .iter()
        .copied()
        .filter(|&i| {
            matches!(&log.events[i], CombatEvent::AnimatedCast(f) if f.actual_duration > 0)
        })
        .chain(swap_rows.iter().copied())
        .collect();
    all.sort_by_key(|&i| (log.events[i].time().unwrap_or(0), is_swap_kind(log, i)));
    let mut swapped: i32 = weapon_set_ids::NO_SET;
    let mut swapped_time: i64 = log.log_data.log_start;
    for &i in &all {
        let evt = &log.events[i];
        let (time, skill_id) = match evt {
            CombatEvent::AnimatedCast(f) => (f.time, f.skill_id),
            CombatEvent::WeaponSwap(f) => (f.time, WEAPON_SWAP_ID as u32),
            _ => continue,
        };
        let is_weapon_swap = matches!(evt, CombatEvent::WeaponSwap(_));
        if !is_weapon_swap && crate::signed_id(skill_id) == WEAPON_SWAP_ID {
            continue;
        }
        // 可推断的技能必须有 descriptor
        let desc = skills
            .get(crate::signed_id(skill_id))
            .and_then(|s| s.descriptor);
        // SkillItem.FindFirstWeaponSet(SkillItem.cs:149-162):swapped 未定
        // 时,首个 swap 事件的 SwappedFrom 若为有效武器组直接采用(与技能
        // 有无 descriptor 无关);否则按 descriptor 的 to 推断。
        if swapped == weapon_set_ids::NO_SET {
            let first_from = swap_rows
                .first()
                .and_then(|&s| match &log.events[s] {
                    CombatEvent::WeaponSwap(f) => Some(i64::from(f.swapped_from) as i32),
                    _ => None,
                });
            let mut initial = weapon_set_ids::NO_SET;
            if let Some(from) = first_from
                && weapon_set_ids::is_weapon_set(from)
            {
                initial = from;
            } else if let Some(d) = desc {
                let first_swap_to = swap_rows
                    .first()
                    .and_then(|&s| match &log.events[s] {
                        CombatEvent::WeaponSwap(f) => Some(i64::from(f.swapped_to) as i32),
                        _ => None,
                    })
                    .unwrap_or(weapon_set_ids::NO_SET);
                let swap_tos: Vec<i32> = swap_rows
                    .iter()
                    .filter_map(|&s| match &log.events[s] {
                        CombatEvent::WeaponSwap(f) => Some(i64::from(f.swapped_to) as i32),
                        _ => None,
                    })
                    .collect();
                initial = d.find_first_weapon_set(first_swap_to, &swap_tos);
            }
            swapped = initial;
        }
        if let Some(d) = desc {
            let valid_for_swap = time > swapped_time + WEAPON_SWAP_DELAY;
            if weapon_set_ids::is_weapon_set(swapped) && valid_for_swap {
                let (api_skill, cur) = match skills.get(crate::signed_id(skill_id)) {
                    Some(s) if s.api.is_some() => {
                        // find current set (last)
                        let cur = sets.last_mut().expect("at least one set");
                        (s.api.clone().expect("api"), cur)
                    }
                    _ => continue,
                };
                let _ = api_skill;
                let _ = cur;
                // 简化评估:不满足 → 新 set 需要(双套推断)
                let ok = try_set_weapon(sets.last_mut().expect("set"), d, &skills.get(crate::signed_id(skill_id)).expect("skill").api.clone().expect("api"), time, swapped);
                if !ok {
                    let mid = (sets.last().expect("set").end + 1 + time) / 2;
                    sets.last_mut().expect("set").end = mid;
                    let mut ns = WeaponSetOut {
                        weapons: vec![
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                            "Unknown".to_string(),
                        ],
                        start: mid,
                        end: log.log_data.log_end,
                    };
                    let mut first = true;
                    for w in &mut ns.weapons {
                        if first {
                            let _ = w;
                            first = false;
                        }
                    }
                    try_set_weapon(&mut ns, d, &skills.get(crate::signed_id(skill_id)).expect("skill").api.clone().expect("api"), time, swapped);
                    sets.push(ns);
                }
            }
            if is_weapon_swap
                && let CombatEvent::WeaponSwap(f) = evt
            {
                swapped = i64::from(f.swapped_to) as i32;
                swapped_time = f.time;
            }
        } else if is_weapon_swap
            && let CombatEvent::WeaponSwap(f) = evt
        {
            swapped = i64::from(f.swapped_to) as i32;
            swapped_time = f.time;
        }
    }
    if let Some(last) = sets.last_mut() {
        last.end = log.log_data.log_end;
    }
    sets
}

const WEAPON_SWAP_DELAY: i64 = 30; // WeaponSwapDelayConstant(占位,见 ArcDPSEnums)

fn try_set_weapon(
    set: &mut WeaponSetOut,
    d: crate::skills::WeaponDescriptor,
    api: &crate::content::ApiSkill,
    _time: i64,
    swapped: i32,
) -> bool {
    use crate::skills::Hand;
    let wt = api.weapon_type.clone().unwrap_or_default();
    let dual = api.dual_wield.clone().unwrap_or_default();
    let (i0, i1) = slot_indices(d.is_land, swapped);
    let set_one = |idx: usize, val: &str, set: &mut WeaponSetOut| -> bool {
        let cur = set.weapons[idx].clone();
        if cur != "Unknown" && cur != val {
            return false;
        }
        set.weapons[idx] = val.to_string();
        true
    };
    match d.slot {
        Hand::MainHand => set_one(i0, &wt, set),
        Hand::OffHand => set_one(i1, &wt, set),
        Hand::TwoHand => set_one(i0, &wt, set) && set_one(i1, "2Hand", set),
        Hand::Dual => {
            if dual.is_empty() || dual == "None" || dual == "Nothing" {
                return true;
            }
            set_one(i0, &wt, set) && set_one(i1, &dual, set)
        }
    }
}

fn slot_indices(is_land: bool, swapped: i32) -> (usize, usize) {
    // [LandMH1, LandOH1, LandMH2, LandOH2, WaterMH1, WaterOH1, WaterMH2, WaterOH2]
    let base = if is_land {
        if swapped == weapon_set_ids::FIRST_LAND { 0 } else { 2 }
    } else if swapped == weapon_set_ids::FIRST_WATER {
        4
    } else {
        6
    };
    (base, base + 1)
}

// ===== consumables / death recap =====

/// SetConsumablesList:分类 Nourishment/Enhancement/OtherConsumable 的 buff
/// apply(to==actor)事件;10ms 内同 id 合并 stack。
pub fn build_consumables(
    ctx: &Ctx,
    agent: AgentId,
    apply_rows: &[(i64, i64)], // (time, applied duration) per buff id 由调用方筛
    buff_ids: &[i64],
) -> Option<Vec<JsonConsumable>> {
    let mut items: Vec<JsonConsumable> = Vec::new();
    for &bid in buff_ids {
        for &(t, dur) in apply_rows {
            if t <= ctx.log_end {
                if let Some(ex) = items
                    .iter_mut()
                    .find(|x| x.id == bid && (x.time - t).abs() < SERVER_DELAY)
                {
                    ex.stack += 1;
                } else {
                    items.push(JsonConsumable { stack: 1, duration: dur, time: t, id: bid });
                }
            }
        }
    }
    items.sort_by_key(|x| x.time);
    let _ = agent;
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// SetDeathRecaps(死亡回放;每 dead 事件一段)。
pub fn build_death_recaps(
    ctx: &Ctx,
    agent: AgentId,
    dead_rows: &[usize],
    down_rows: &[usize],
    up_rows: &[usize],
    last_death_anchor: &mut i64,
) -> Vec<JsonDeathRecap> {
    let log = ctx.log;
    let mut out = Vec::new();
    for &d in dead_rows {
        let dead_time = log.events[d].time().unwrap_or(0);
        let last_death = *last_death_anchor;
        *last_death_anchor = dead_time;
        // damageLogs:该 actor 的承伤行(全时段)
        let taken = ctx
            .rows
            .health_by_to
            .get(&agent)
            .cloned()
            .unwrap_or_default();
        let upped = up_rows
            .iter()
            .filter(|&&e| {
                let t = log.events[e].time().unwrap_or(0);
                t <= dead_time && t >= last_death
            })
            .map(|&e| log.events[e].time().unwrap_or(0))
            .next_back();
        let downed = match upped {
            Some(up_t) => down_rows
                .iter()
                .filter(|&&e| {
                    let t = log.events[e].time().unwrap_or(0);
                    t <= dead_time && t >= up_t
                })
                .map(|&e| log.events[e].time().unwrap_or(0))
                .next_back(),
            None => down_rows
                .iter()
                .filter(|&&e| {
                    let t = log.events[e].time().unwrap_or(0);
                    t <= dead_time && t >= last_death
                })
                .map(|&e| log.events[e].time().unwrap_or(0))
                .next_back(),
        };
        let mut recap = JsonDeathRecap { death_time: dead_time, to_down: None, to_kill: None };
        if let Some(down_t) = downed {
            // toDown:lastDeath < t <= downed.Time && (HasHit || HasDowned)
            let mut dmg_to_down: Vec<(i64, i64)> = taken
                .iter()
                .filter(|hl| {
                    let Some(row) = stats::row_view(ctx, hl) else { return false };
                    hl.line.time > last_death
                        && hl.line.time <= down_t
                        && (row.has_hit || row.has_downed)
                })
                .map(|hl| (hl.line.time, hl.line.ev as i64))
                .collect();
            if !dmg_to_down.is_empty() {
                dmg_to_down.sort_by_key(|x| x.0);
                let mut items = Vec::new();
                let mut damage = 0i64;
                for &(_, evi) in dmg_to_down.iter().rev() {
                    items.push(recap_item(log, evi as usize, agent));
                    damage += recap_damage(log, evi as usize);
                    if damage > 20_000 {
                        break;
                    }
                }
                recap.to_down = Some(items);
            }
            let mut dmg_to_kill: Vec<(i64, i64)> = taken
                .iter()
                .filter(|hl| {
                    let Some(row) = stats::row_view(ctx, hl) else { return false };
                    hl.line.time > down_t
                        && hl.line.time <= dead_time
                        && (row.has_hit || row.has_killed)
                })
                .map(|hl| (hl.line.time, hl.line.ev as i64))
                .collect();
            if !dmg_to_kill.is_empty() {
                dmg_to_kill.sort_by_key(|x| x.0);
                let items: Vec<JsonDeathRecapItem> = dmg_to_kill
                    .iter()
                    .rev()
                    .map(|&(_, evi)| recap_item(log, evi as usize, agent))
                    .collect();
                recap.to_kill = Some(items);
            }
        } else {
            let mut dmg_to_kill: Vec<(i64, i64)> = taken
                .iter()
                .filter(|hl| {
                    let Some(row) = stats::row_view(ctx, hl) else { return false };
                    hl.line.time > last_death
                        && hl.line.time <= dead_time
                        && (row.has_hit || row.has_killed)
                })
                .map(|hl| (hl.line.time, hl.line.ev as i64))
                .collect();
            if !dmg_to_kill.is_empty() {
                dmg_to_kill.sort_by_key(|x| x.0);
                let mut items = Vec::new();
                let mut damage = 0i64;
                for &(_, evi) in dmg_to_kill.iter().rev() {
                    items.push(recap_item(log, evi as usize, agent));
                    damage += recap_damage(log, evi as usize);
                    if damage > 20_000 {
                        break;
                    }
                }
                recap.to_kill = Some(items);
            }
        }
        out.push(recap);
    }
    out
}

fn recap_damage(log: &ParsedLog, evi: usize) -> i64 {
    match &log.events[evi] {
        CombatEvent::DirectHealthDamage(f)
        | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => i64::from(f.health_damage),
        _ => 0,
    }
}

fn recap_item(log: &ParsedLog, evi: usize, agent: AgentId) -> JsonDeathRecapItem {
    let (id, indirect, src_addr, damage, time) = match &log.events[evi] {
        CombatEvent::DirectHealthDamage(f) | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => (
            crate::signed_id(f.skill.skill_id),
            matches!(&log.events[evi], CombatEvent::NonDirectHealthDamage(_)),
            f.skill.from,
            i64::from(f.health_damage),
            f.skill.time,
        ),
        _ => (0, false, 0, 0, 0),
    };
    // C# SourceAgent = log.FindActor(from).Character;找不到时用 actor 名
    let mut t = log.agents.clone();
    let from_id = if src_addr != 0 {
        t.resolve_agent(src_addr, time)
    } else {
        NO_AGENT
    };
    let char = if from_id != NO_AGENT && from_id != agent {
        log.agents.slot(from_id).map(|a| a.character.clone()).unwrap_or_default()
    } else {
        "UNKNOWN".to_string()
    };
    JsonDeathRecapItem { id, indirect_damage: indirect, src: char, damage, time }
}
