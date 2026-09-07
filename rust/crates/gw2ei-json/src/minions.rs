//! minions 块（P4）—— 玩家（含友方非小队）的 minion 聚合 JSON。
//!
//! C#：SingleActor.GetMinions(SingleActor.cs:135-202) 分组（master 链终主 =
//! 玩家且 aware 相交;StableSpecies 按物种 id、VolatileSpecies 按 agent.Name
//! 聚合;`Minions.IsActive` 过滤）+ JsonMinionsBuilder.cs。
//!
//! 有意偏差登记：`IsKnownMinionID` 职业表（ProfHelper 系）未迁移 ——
//! IsActive 用「有伤害行 || 有 cast 行 || Ranger 幼宠（icons.json
//! uniqueMinions）」近似;C# 的 EXT heal/barrier 存在性判据随 EXT 整体不迁移
//!（样例无 EXT 数据）。IsUniquePerTimeFrame 仅实现 Ranger 幼宠面
//!（Minions.cs:33-36 的 Mechanist/Necro/Ritualist 集合同理可扩）。

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, AgentType};

use crate::actors::{ActorBuilder, CastRef, Collectors};
use crate::ctx::Ctx;
use crate::dto_b::JsonMinions;
use crate::rows::{ActorRows, HealthLine};
use crate::skills::SkillTable;

/// 一组 minion（同物种/同名的实例集合;顺序 = 首实例 FirstAware 序）。
struct Group {
    /// 实例 agent id（首实例 = 参考;C# `_minionList`）。
    instances: Vec<AgentId>,
    /// 组名（首实例 Character;JsonMinions.Name）。
    name: String,
    /// 组 key：StableSpecies = 物种 id;VolatileSpecies 按名字聚合 → -1。
    id: i64,
    is_unique: bool,
}

/// 构建某 master 的 minion 组（C# GetMinions;组序 = agent FirstAware 首现序）。
fn groups_of(ctx: &Ctx, master: AgentId) -> Vec<Group> {
    let log = ctx.log;
    let Some(m) = log.agents.slot(master) else { return Vec::new() };
    let master_root = log.agents.englobing_root(master);
    let (mfa, mla) = (m.first_aware, m.last_aware);
    // C# GetAgentByType 遍历按 FirstAware 升序(stable 组先于 volatile 组)。
    let ids = log.agents.ids_sorted_by_first_aware();
    let mut stable: Vec<(i64, Group)> = Vec::new();
    let mut volatile: Vec<(String, Group)> = Vec::new();
    for id in ids {
        let Some(a) = log.agents.slot(id) else { continue };
        if a.agent_type != AgentType::StableSpecies && a.agent_type != AgentType::VolatileSpecies {
            continue;
        }
        // IsMasterOf:minion 终主 == master 且非自身
        let fm = log.agents.final_master(id);
        if fm == id || log.agents.englobing_root(fm) != master_root {
            continue;
        }
        // InAwareTimes(AgentItem):aware 相交
        if !(a.first_aware <= mla && a.last_aware >= mfa) {
            continue;
        }
        if a.agent_type == AgentType::StableSpecies {
            let key = i64::from(a.id);
            match stable.iter_mut().find(|(k, _)| *k == key) {
                Some((_, g)) => g.instances.push(id),
                None => {
                    stable.push((
                        key,
                        Group {
                            instances: vec![id],
                            name: a.character.clone(),
                            id: key,
                            is_unique: ctx.content.icons.ranger_juvenile.contains(&key),
                        },
                    ));
                }
            }
        } else {
            // VolatileSpecies 按 agent.Name 聚合;组 id = 参考实例物种 id
            // （C# Minions.ID = ReferenceAgentItem.ID;炮台 4933 等）。
            let key = a.name.clone();
            match volatile.iter_mut().find(|(k, _)| *k == key) {
                Some((_, g)) => g.instances.push(id),
                None => {
                    volatile.push((
                        key.clone(),
                        Group {
                            instances: vec![id],
                            name: a.character.clone(),
                            id: i64::from(a.id),
                            is_unique: false,
                        },
                    ));
                }
            }
        }
    }
    let mut out: Vec<Group> = Vec::new();
    for (_, g) in stable {
        out.push(g);
    }
    for (_, g) in volatile {
        out.push(g);
    }
    out
}

/// Minions.IsActive（Minions.cs:120-150）的近似:known-minion(按 master
/// 基专精:Ranger/Mesmer 集已抽取;其余职业表未迁移 → 伤害/cast 兜底)||
/// 伤害行 || cast 行(排除 WeaponStow/WeaponDraw/WeaponSwap/MirageCloakDodge)。
fn group_active(ctx: &Ctx, g: &Group, master_base_spec: &str) -> bool {
    if g.is_unique {
        return true;
    }
    let known = match master_base_spec {
        "Ranger" => &ctx.content.icons.ranger_known,
        "Mesmer" => &ctx.content.icons.mesmer_known,
        _ => return false,
    };
    if known.contains(&g.id) {
        return true;
    }
    let log = ctx.log;
    for &inst in &g.instances {
        let actor = ActorRows::new(log, &ctx.rows, inst);
        if !actor.health_out(0, log.log_data.log_end, None).is_empty() {
            return true;
        }
    }
    for &inst in &g.instances {
        if let Some(list) = ctx.aux.cast_by_caster.get(&inst) {
            for &e in list {
                let sid = match &log.events[e] {
                    CombatEvent::AnimatedCast(f) => crate::signed_id(f.skill_id),
                    CombatEvent::Emote(f) => crate::signed_id(f.base.skill_id),
                    CombatEvent::GadgetInteract(f) => crate::signed_id(f.skill_id),
                    CombatEvent::BundlePickUp(f) => crate::signed_id(f.base.skill_id),
                    _ => 0,
                };
                // WeaponStow=23285/WeaponDraw=23284/WeaponSwap=-2/MirageCloakDodge=-17
                let excluded = matches!(sid, 23284 | 23285 | -2 | -17);
                if !excluded {
                    return true;
                }
            }
        }
    }
    false
}

/// 玩家 minions JSON（JsonActorBuilder.cs:57-61 的副作用:技能注册进 skillMap
/// 在 dist/rotation 收集器内;buffMap 不经此路径）。
pub fn build_minions(
    ctx: &Ctx,
    skills: &SkillTable,
    cols: &mut Collectors,
    master: AgentId,
) -> Option<Vec<JsonMinions>> {
    let log = ctx.log;
    let phase_start = log.log_data.log_start;
    let phase_end = log.log_data.log_end;
    let target_id = log.log_data.dummy_target_agent;
    let can_crit = |id: i64| crate::content::skill_can_crit(ctx.content, id, ctx.meta.gw2_build);

    let groups = groups_of(ctx, master);
    if groups.is_empty() {
        return None;
    }
    let mut out: Vec<JsonMinions> = Vec::new();
    let base_spec = log
        .agents
        .slot(master)
        .map(|a| a.base_spec.csharp_name().to_string())
        .unwrap_or_default();
    for g in groups {
        if !group_active(ctx, &g, &base_spec) {
            continue;
        }
        // ---- 伤害行(各实例;health_out 语义含自身 minion 链;窗口过滤)----
        let mut out_rows: Vec<HealthLine> = Vec::new();
        let mut out_taken: Vec<HealthLine> = Vec::new();
        let mut out_target: Vec<HealthLine> = Vec::new();
        let mut bb_out: Vec<crate::rows::DmgLine> = Vec::new();
        let mut bb_taken: Vec<crate::rows::DmgLine> = Vec::new();
        let mut bb_target: Vec<crate::rows::DmgLine> = Vec::new();
        let dummy_root = log.agents.englobing_root(target_id);
        for &inst in &g.instances {
            let actor = ActorRows::new(log, &ctx.rows, inst);
            out_rows.extend(actor.health_out(phase_start, phase_end, None));
            out_taken.extend(actor.health_in(phase_start, phase_end, None));
            out_target.extend(actor.health_out(phase_start, phase_end, Some(target_id)));
            bb_out.extend(actor.breakbar_out(phase_start, phase_end));
            bb_taken.extend(actor.breakbar_in(phase_start, phase_end));
            for l in actor.breakbar_out(phase_start, phase_end) {
                if log.agents.englobing_root(l.to) == dummy_root {
                    bb_target.push(l);
                }
            }
        }
        // C# 组内行按时间稳定排序(SortByTime)
        let sort_key = |hl: &HealthLine| hl.line.time;
        out_rows.sort_by_key(sort_key);
        out_taken.sort_by_key(sort_key);
        out_target.sort_by_key(sort_key);
        let bb_key = |l: &crate::rows::DmgLine| l.time;
        bb_out.sort_by_key(bb_key);
        bb_taken.sort_by_key(bb_key);
        bb_target.sort_by_key(bb_key);

        // ---- per-phase 总量(单 phase)----
        let sum_hp = |rows: &[HealthLine]| -> i64 {
            rows.iter()
                .filter_map(|hl| crate::stats::row_view(ctx, hl))
                .map(|r| i64::from(r.health_damage))
                .sum()
        };
        let sum_sh = |rows: &[HealthLine]| -> i64 {
            rows.iter()
                .filter_map(|hl| crate::stats::row_view(ctx, hl))
                .map(|r| i64::from(r.shield_damage))
                .sum()
        };
        let sum_bb = |rows: &[crate::rows::DmgLine]| -> f64 {
            let s: f64 = rows
                .iter()
                .map(|l| {
                    match &log.events[l.ev] {
                        CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => f.value,
                        _ => 0.0,
                    }
                })
                .sum();
            (s * 10.0).round_ties_even() / 10.0
        };

        // ---- dist(组内聚合行;downContribution null —— C# JsonMinionsBuilder)----
        let dist_out =
            crate::actors::dist_from_lines(ctx, skills, cols, &out_rows, &bb_out, &can_crit);
        let dist_taken =
            crate::actors::dist_from_lines(ctx, skills, cols, &out_taken, &bb_taken, &can_crit);
        let dist_target =
            crate::actors::dist_from_lines(ctx, skills, cols, &out_target, &bb_target, &can_crit);

        // ---- rotation(组内 cast 行;P3b 合成引擎不含 minion 面 —— C# 的
        // minion rotation 纯 animated cast)----
        let mut cast_refs: Vec<CastRef> = Vec::new();
        for &inst in &g.instances {
            if let Some(list) = ctx.aux.cast_by_caster.get(&inst) {
                cast_refs.extend(list.iter().copied().map(CastRef::Ev));
            }
        }
        let rotation = crate::actors::build_rotation(
            &ActorBuilder { ctx, actor: ActorRows::new(log, &ctx.rows, g.instances[0]) },
            phase_start,
            phase_end,
            skills,
            cols,
            &cast_refs,
        );

        // ---- combatReplayData 逐实例 ----
        let cr_list: Vec<crate::dto_b::JsonActorCombatReplayData> = if crate::replay::can_combat_replay(ctx) {
            g.instances
                .iter()
                .map(|&inst| {
                    let icon = crate::replay::actor_icon(ctx, inst);
                    crate::replay::actor_cr_data(ctx, inst, &icon)
                })
                .collect()
        } else {
            Vec::new()
        };
        let cr = if cr_list.is_empty() { None } else { Some(cr_list) };

        out.push(JsonMinions {
            name: g.name,
            id: g.id,
            total_damage: vec![sum_hp(&out_rows)],
            total_target_damage: vec![vec![sum_hp(&out_target)]],
            total_damage_taken: vec![sum_hp(&out_taken)],
            total_breakbar_damage: vec![sum_bb(&bb_out)],
            total_target_breakbar_damage: vec![vec![sum_bb(&bb_target)]],
            total_breakbar_damage_taken: vec![sum_bb(&bb_taken)],
            total_shield_damage: vec![sum_sh(&out_rows)],
            total_target_shield_damage: vec![vec![sum_sh(&out_target)]],
            total_shield_damage_taken: vec![sum_sh(&out_taken)],
            total_damage_dist: vec![dist_out],
            target_damage_dist: vec![vec![dist_target]],
            total_damage_taken_dist: vec![dist_taken],
            rotation,
            is_unique_per_time_frame: g.is_unique,
            combat_replay_data: cr,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
