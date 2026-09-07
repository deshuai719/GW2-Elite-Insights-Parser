//! JSON buff 族块组装（P3a）：buffUptimes×2、self/group/offGroup/squad
//! Buffs×2 生成族、boonsStates/conditionsStates、avgBoons 系。
//!
//! 公式/形状锚点：JsonPlayerBuilder.cs:148-…、JsonBuffsUptimeBuilder.cs、
//! JsonPlayerBuffsGenerationBuilder.cs、BuffStatistics.cs、SingleActorBuffs
//! Helper.cs（伪图）、GameplayStatistics.cs:121-136。

use std::collections::{BTreeMap, BTreeSet};

use gw2ei_model::{AgentId, NO_AGENT, ParsedLog};
use gw2ei_semantics::stats::{self, by_actor, round3, srcs_of};

use crate::buffsim::{ActorSims, BuffSims, OneBuff};
use crate::content::BuffClassification;
use crate::dto_b::{
    JsonBuffsGenerationData, JsonBuffsUptime, JsonBuffsUptimeData, JsonPlayerBuffsGeneration,
};

/// 单 player 的 buff 族 DTO 全部输出（build_player 直接填 JsonPlayer）。
pub struct PlayerBuffBlocks {
    pub buff_uptimes: Vec<JsonBuffsUptime>,
    pub buff_uptimes_active: Vec<JsonBuffsUptime>,
    pub self_buffs: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub self_buffs_active: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub group_buffs: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub group_buffs_active: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub off_group_buffs: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub off_group_buffs_active: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub squad_buffs: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub squad_buffs_active: Option<Vec<JsonPlayerBuffsGeneration>>,
    pub conditions_states: Vec<Vec<i64>>,
    pub boons_states: Vec<Vec<i64>>,
    pub avg_boons: f64,
    pub avg_active_boons: f64,
    pub avg_conditions: f64,
    pub avg_active_conditions: f64,
    /// 进入 buffMap 的 id（C# 各 builder 的副作用收口）。
    pub map_ids: BTreeSet<i64>,
}

/// 源 actor 的 JSON 名（C# Character 面）。
pub fn src_name(log: &ParsedLog, key: AgentId) -> String {
    if key == NO_AGENT {
        return "UNKNOWN".to_string();
    }
    let Some(a) = log.agents.slot(key) else {
        return "UNKNOWN".to_string();
    };
    match a.agent_type {
        gw2ei_model::AgentType::Player => a.character.clone(),
        gw2ei_model::AgentType::NonSquadPlayer => {
            format!("{} pl-{}", a.spec.csharp_name(), a.instid)
        }
        _ => a.character.clone(),
    }
}

fn hidden(one: &OneBuff) -> bool {
    one.buff.classification == BuffClassification::Hidden
}

/// item 段（whole log；无 per_src 时 = active stacks）。
fn item_segments(one: &OneBuff, per_src: Option<AgentId>) -> Vec<(i64, i64, i64)> {
    one.result
        .items
        .iter()
        .map(|item| {
            let v: i64 = match per_src {
                None => item.groups.iter().map(|g| g.count).sum(),
                Some(by) => item.groups.iter().filter(|g| g.src == by).map(|g| g.count).sum(),
            };
            (item.start, item.end, v)
        })
        .collect()
}

fn graph_states(
    one: &OneBuff,
    per_src: Option<AgentId>,
    log_start: i64,
    log_end: i64,
) -> Vec<Vec<i64>> {
    let segs = item_segments(one, per_src);
    let graph = if segs.is_empty() {
        vec![(log_start, log_end, 0)]
    } else {
        stats::fuse_segments(log_start, log_end, segs)
    };
    graph.iter().map(|&(s, _, v)| vec![s, v]).collect()
}

/// per-src 7 向字典（值已 round3）。
type Seven = [BTreeMap<String, f64>; 7];

fn by_src_dicts(
    log: &ParsedLog,
    srcs: &[AgentId],
    rates: &BTreeMap<AgentId, stats::PerSrcStats>,
) -> Seven {
    let mut maps: Seven = std::array::from_fn(|_| BTreeMap::new());
    for &src in srcs {
        let Some(r) = rates.get(&src) else { continue };
        let name = src_name(log, src);
        maps[0].insert(name.clone(), r.generated);
        maps[1].insert(name.clone(), r.generated_presence);
        maps[2].insert(name.clone(), r.overstacked);
        maps[3].insert(name.clone(), r.wasted);
        maps[4].insert(name.clone(), r.unknown_extended);
        maps[5].insert(name.clone(), r.by_extension);
        maps[6].insert(name, r.extended);
    }
    maps
}

/// 组装（单 phase = whole log）。
pub fn build_player_blocks(
    ctx: &crate::ctx::Ctx<'_>,
    store: &BuffSims,
    actor: AgentId,
    group: i64,
    in_squad: bool,
) -> PlayerBuffBlocks {
    let log = ctx.log;
    let log_start = log.log_data.log_start;
    let log_end = log.log_data.evtc_log_end;
    let phase_duration = log_end - log_start;
    let actor_sims = store.per_actor.get(&actor).expect("actor sims");
    let mut map_ids: BTreeSet<i64> = BTreeSet::new();

    // ---- buffUptimes / buffUptimesActive（GetBuffs(Self) 面）----
    let mut buff_uptimes: Vec<JsonBuffsUptime> = Vec::new();
    let mut buff_uptimes_active: Vec<JsonBuffsUptime> = Vec::new();
    for (&id, one) in &actor_sims.by_buff {
        if hidden(one) {
            continue;
        }
        if one.distrib.by_src.is_empty() {
            continue; // HasBuffID 面：phase 内无贡献
        }
        map_ids.insert(id);
        let (data, data_active) = uptime_data(log, one, actor, phase_duration, actor_sims.active_duration);
        let states = graph_states(one, None, log_start, log_end);
        let mut states_per_source = BTreeMap::new();
        for src in srcs_of(&one.distrib) {
            states_per_source.insert(src_name(log, src), graph_states(one, Some(src), log_start, log_end));
        }
        buff_uptimes.push(JsonBuffsUptime {
            id,
            buff_data: vec![data],
            states: Some(states),
            states_per_source: Some(states_per_source),
        });
        buff_uptimes_active.push(JsonBuffsUptime {
            id,
            buff_data: vec![data_active],
            states: Some(graph_states(one, None, log_start, log_end)),
            states_per_source: Some(
                srcs_of(&one.distrib)
                    .into_iter()
                    .map(|s| (src_name(log, s), graph_states(one, Some(s), log_start, log_end)))
                    .collect(),
            ),
        });
    }

    // ---- 生成族（self = 自身；group/offGroup/squad = PlayerList 子集）----
    // Self：GetBuffsForSelf 的 7 数字（键域 = buffUptimes 键域）
    let mut self_buffs: Vec<JsonPlayerBuffsGeneration> = Vec::new();
    let mut self_buffs_active: Vec<JsonPlayerBuffsGeneration> = Vec::new();
    for entry in &buff_uptimes {
        let id = entry.id;
        let one = actor_sims.by_buff.get(&id).expect("entry buff");
        let (data, data_active) = self_generation(one, actor, phase_duration, actor_sims.active_duration);
        self_buffs.push(JsonPlayerBuffsGeneration {
            id,
            buff_data: vec![data],
        });
        self_buffs_active.push(JsonPlayerBuffsGeneration {
            id,
            buff_data: vec![data_active],
        });
    }
    // Group/OffGroup/Squad（PlayerList 派生；squadless Player 的 Group 已归一）
    let players: Vec<(AgentId, i64)> = log
        .players
        .iter()
        .map(|p| (p.agent, i64::from(p.group)))
        .collect();
    let other_players: Vec<AgentId> = players.iter().filter(|(a, _)| *a != actor).map(|(a, _)| *a).collect();
    let same_group: Vec<AgentId> = if in_squad {
        players
            .iter()
            .filter(|(a, g)| *a != actor && *g == group)
            .map(|(a, _)| *a)
            .collect()
    } else {
        Vec::new()
    };
    let off_group: Vec<AgentId> = if in_squad {
        players
            .iter()
            .filter(|(a, g)| *a != actor && *g != group)
            .map(|(a, _)| *a)
            .collect()
    } else {
        Vec::new()
    };
    // NonSquad querier：Group/OffGroup 恒空；Squad = 全部 PlayerList
    let squad: Vec<AgentId> = other_players.clone();
    let mut fam = |scope: &[AgentId]| -> (Option<Vec<JsonPlayerBuffsGeneration>>, Option<Vec<JsonPlayerBuffsGeneration>>) {
        let (plain, active, ids) = generation_family(ctx, store, actor, scope, phase_duration);
        map_ids.extend(ids);
        let opt = |v: Vec<JsonPlayerBuffsGeneration>| if v.is_empty() { None } else { Some(v) };
        (opt(plain), opt(active))
    };
    let (group_buffs, group_buffs_active) = fam(&same_group);
    let (off_group_buffs, off_group_buffs_active) = fam(&off_group);
    let (squad_buffs, squad_buffs_active) = fam(&squad);

    // ---- boonsStates / conditionsStates（伪图 = 同分类 presence 叠加）----
    let conditions_states = pseudo_states(actor_sims, BuffClassification::Condition, log_start, log_end);
    let boons_states = pseudo_states(actor_sims, BuffClassification::Boon, log_start, log_end);
    // ---- avgBoons 系 ----
    let (avg_boons, avg_active_boons) = avg_boons_conds(
        actor_sims,
        BuffClassification::Boon,
        phase_duration,
        actor_sims.active_duration,
    );
    let (avg_conditions, avg_active_conditions) = avg_boons_conds(
        actor_sims,
        BuffClassification::Condition,
        phase_duration,
        actor_sims.active_duration,
    );

    PlayerBuffBlocks {
        buff_uptimes,
        buff_uptimes_active,
        self_buffs: if self_buffs.is_empty() { None } else { Some(self_buffs) },
        self_buffs_active: if self_buffs_active.is_empty() { None } else { Some(self_buffs_active) },
        group_buffs,
        group_buffs_active,
        off_group_buffs,
        off_group_buffs_active,
        squad_buffs,
        squad_buffs_active,
        conditions_states,
        boons_states,
        avg_boons,
        avg_active_boons,
        avg_conditions,
        avg_active_conditions,
        map_ids,
    }
}

/// BuffStatistics.GetBuffsForSelf（BuffStatistics.cs:161-251）。
fn uptime_data(
    log: &ParsedLog,
    one: &OneBuff,
    actor: AgentId,
    phase_duration: i64,
    active_duration: i64,
) -> (JsonBuffsUptimeData, JsonBuffsUptimeData) {
    let d = &one.distrib;
    let (s, sa) = stats::self_stats(
        one.intensity,
        d,
        d.presence_ms,
        d.presence_of(actor),
        phase_duration,
        active_duration,
        actor,
    );
    let srcs = srcs_of(d);
    let (rates, rates_active) = by_actor(
        one.intensity,
        d,
        &srcs,
        &d.presence_by,
        phase_duration,
        active_duration,
    );
    let [g, gp, o, w, u, b, e] = by_src_dicts(log, &srcs, &rates);
    let [ga, gpa, oa, wa, ua, ba, ea] = by_src_dicts(log, &srcs, &rates_active);
    (
        JsonBuffsUptimeData {
            uptime: s.uptime,
            presence: s.presence,
            generated: g,
            generated_presence: gp,
            overstacked: o,
            wasted: w,
            unknown_extended: u,
            by_extension: b,
            extended: e,
        },
        JsonBuffsUptimeData {
            uptime: sa.uptime,
            presence: sa.presence,
            generated: ga,
            generated_presence: gpa,
            overstacked: oa,
            wasted: wa,
            unknown_extended: ua,
            by_extension: ba,
            extended: ea,
        },
    )
}

/// BuffByActorStatistics（BuffByActorStatistics.cs:20-83）的 self 侧 7 数字
///（= per-src of self；JSON generation 族 data 源）。
fn self_generation(
    one: &OneBuff,
    actor: AgentId,
    phase_duration: i64,
    active_duration: i64,
) -> (JsonBuffsGenerationData, JsonBuffsGenerationData) {
    let d = &one.distrib;
    let (s, sa) = stats::self_stats(
        one.intensity,
        d,
        d.presence_ms,
        d.presence_of(actor),
        phase_duration,
        active_duration,
        actor,
    );
    let to_data = |s: &stats::SelfStats| JsonBuffsGenerationData {
        generation: s.generation,
        generation_presence: s.generation_presence,
        overstack: s.overstack,
        wasted: s.wasted,
        unknown_extended: s.unknown_extended,
        extended: s.extended,
        by_extension: s.by_extension,
    };
    (to_data(&s), to_data(&sa))
}

/// GetBuffsForPlayers（BuffStatistics.cs:18-158）：querier 为源、scope 为
/// 玩家集（C# PlayerList 子集；已按 group 圈定）。返回 (buffs, activeBuffs,
/// 有 hasGeneration 的 buff id 集)。
fn generation_family(
    ctx: &crate::ctx::Ctx<'_>,
    store: &BuffSims,
    querier: AgentId,
    scope: &[AgentId],
    phase_duration: i64,
) -> (
    Vec<JsonPlayerBuffsGeneration>,
    Vec<JsonPlayerBuffsGeneration>,
    BTreeSet<i64>,
) {
    // 各 scope player 分布键集合并：query id = 出现且 querier 为源者
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    for &p in scope {
        let Some(psims) = store.per_actor.get(&p) else { continue };
        for (&id, one) in &psims.by_buff {
            if hidden(one) {
                continue;
            }
            if one.distrib.by_src.contains_key(&querier) {
                ids.insert(id);
            }
        }
    }
    let mut plain = Vec::new();
    let mut active = Vec::new();
    for &id in &ids {
        let (d, da) = generation_phase_data(ctx, store, querier, id, scope, phase_duration);
        plain.push(JsonPlayerBuffsGeneration {
            id,
            buff_data: vec![d],
        });
        active.push(JsonPlayerBuffsGeneration {
            id,
            buff_data: vec![da],
        });
    }
    (plain, active, ids)
}

/// GetBuffsForPlayers 的单 buff 数值（BuffStatistics.cs:18-158）。
#[allow(clippy::too_many_arguments)]
fn generation_phase_data(
    ctx: &crate::ctx::Ctx<'_>,
    store: &BuffSims,
    querier: AgentId,
    id: i64,
    scope: &[AgentId],
    phase_duration: i64,
) -> (JsonBuffsGenerationData, JsonBuffsGenerationData) {
    let _ = ctx;
    let mut player_count: i64 = 0;
    let mut active_player_count: i64 = 0;
    let mut total_generation = 0.0f64;
    let mut total_generation_presence = 0.0f64;
    let mut total_overstack = 0.0f64;
    let mut total_wasted = 0.0f64;
    let mut total_unknown = 0.0f64;
    let mut total_extension = 0.0f64;
    let mut total_extended = 0.0f64;
    let mut total_active_generation = 0.0f64;
    let mut total_active_generation_presence = 0.0f64;
    let mut total_active_overstack = 0.0f64;
    let mut total_active_wasted = 0.0f64;
    let mut total_active_unknown = 0.0f64;
    let mut total_active_extension = 0.0f64;
    let mut total_active_extended = 0.0f64;
    let mut intensity = false;
    for &p in scope {
        let Some(psims) = store.per_actor.get(&p) else { continue };
        // InAwareTimes(0, logEnd) 恒真（friend aware ⊂ log）；分母仍计入
        player_count += 1;
        let p_active = psims.active_duration;
        let Some(one) = psims.by_buff.get(&id) else { continue };
        if one.distrib.by_src.is_empty() {
            continue; // HasBuffID 门控
        }
        intensity = one.intensity;
        let d = &one.distrib;
        if p_active > 0 {
            active_player_count += 1;
        }
        let generation = d.generation_of(querier) as f64;
        let generation_presence = if one.intensity {
            d.presence_of(querier) as f64
        } else {
            0.0
        };
        let overstack = d.overstack_of(querier) as f64;
        let wasted = d.waste_of(querier) as f64;
        let unknown_extension = d.unknown_ext_of(querier) as f64;
        let extension = d.extension_of(querier) as f64;
        let extended = d.extended_of(querier) as f64;
        total_generation += generation;
        total_generation_presence += generation_presence;
        total_overstack += overstack;
        total_wasted += wasted;
        total_unknown += unknown_extension;
        total_extension += extension;
        total_extended += extended;
        if p_active > 0 {
            total_active_generation += generation / p_active as f64;
            total_active_generation_presence += generation_presence / p_active as f64;
            total_active_overstack += overstack / p_active as f64;
            total_active_wasted += wasted / p_active as f64;
            total_active_unknown += unknown_extension / p_active as f64;
            total_active_extension += extension / p_active as f64;
            total_active_extended += extended / p_active as f64;
        }
    }
    let phase_d = phase_duration as f64;
    let n = player_count as f64;
    let na = active_player_count as f64;
    let mut d = JsonBuffsGenerationData::zero();
    let mut da = JsonBuffsGenerationData::zero();
    if player_count == 0 {
        return (d, da);
    }
    if intensity {
        d.generation = round3(total_generation / phase_d / n);
        d.generation_presence = round3(100.0 * total_generation_presence / phase_d / n);
        d.overstack = round3((total_overstack + total_generation) / phase_d / n);
        d.wasted = round3(total_wasted / phase_d / n);
        d.unknown_extended = round3(total_unknown / phase_d / n);
        d.by_extension = round3(total_extension / phase_d / n);
        d.extended = round3(total_extended / phase_d / n);
        if active_player_count > 0 {
            da.generation = round3(total_active_generation / na);
            da.generation_presence = round3(100.0 * total_active_generation_presence / na);
            da.overstack = round3((total_active_overstack + total_active_generation) / na);
            da.wasted = round3(total_active_wasted / na);
            da.unknown_extended = round3(total_active_unknown / na);
            da.by_extension = round3(total_active_extension / na);
            da.extended = round3(total_active_extended / na);
        }
    } else {
        d.generation = round3(100.0 * total_generation / phase_d / n);
        d.overstack = round3(100.0 * (total_overstack + total_generation) / phase_d / n);
        d.wasted = round3(100.0 * total_wasted / phase_d / n);
        d.unknown_extended = round3(100.0 * total_unknown / phase_d / n);
        d.by_extension = round3(100.0 * total_extension / phase_d / n);
        d.extended = round3(100.0 * total_extended / phase_d / n);
        if active_player_count > 0 {
            da.generation = round3(100.0 * total_active_generation / na);
            da.overstack = round3(100.0 * (total_active_overstack + total_active_generation) / na);
            da.wasted = round3(100.0 * total_active_wasted / na);
            da.unknown_extended = round3(100.0 * total_active_unknown / na);
            da.by_extension = round3(100.0 * total_active_extension / na);
            da.extended = round3(100.0 * total_active_extended / na);
        }
    }
    (d, da)
}

impl JsonBuffsGenerationData {
    fn zero() -> Self {
        JsonBuffsGenerationData {
            generation: 0.0,
            generation_presence: 0.0,
            overstack: 0.0,
            wasted: 0.0,
            unknown_extended: 0.0,
            extended: 0.0,
            by_extension: 0.0,
        }
    }
}

/// avgBoons/avgConditions（GameplayStatistics.cs:121-136）。
pub fn avg_boons_conds(
    actor_sims: &ActorSims,
    kind: BuffClassification,
    phase_duration: i64,
    active_duration: i64,
) -> (f64, f64) {
    let mut sum: i64 = 0;
    for one in actor_sims.by_buff.values() {
        if one.buff.classification == kind {
            sum += one.distrib.presence_ms;
        }
    }
    let avg = round3(sum as f64 / phase_duration as f64);
    let avg_active = if active_duration > 0 {
        round3(sum as f64 / active_duration as f64)
    } else {
        0.0
    };
    (avg, avg_active)
}

/// 伪图 states：同分类 buff presence overlay（SingleActorBuffsHelper 的
/// MergePresenceInto —— 等价面：全图 presence 01 分段 + 边界切分求和 + 融合）。
/// 无该分类仿真 → []。
fn pseudo_states(
    actor_sims: &ActorSims,
    kind: BuffClassification,
    log_start: i64,
    log_end: i64,
) -> Vec<Vec<i64>> {
    let mut layers: Vec<Vec<(i64, i64, i64)>> = Vec::new();
    for one in actor_sims.by_buff.values() {
        if one.buff.classification != kind {
            continue;
        }
        let graph = graph_segments(one, log_start, log_end);
        layers.push(graph);
    }
    if layers.is_empty() {
        return Vec::new();
    }
    let mut bounds: BTreeSet<i64> = BTreeSet::new();
    bounds.insert(log_start);
    bounds.insert(log_end);
    for layer in &layers {
        for &(s, e, _) in layer {
            bounds.insert(s);
            bounds.insert(e);
        }
    }
    let pts: Vec<i64> = bounds.into_iter().collect();
    let mut out: Vec<(i64, i64, i64)> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut v: i64 = 0;
        for layer in &layers {
            for &(s, e, val) in layer {
                if s <= a && b <= e {
                    if val > 0 {
                        v += 1;
                    }
                    break;
                }
            }
        }
        out.push((a, b, v));
    }
    stats::fuse_segments(log_start, log_end, out)
        .iter()
        .map(|&(s, _, v)| vec![s, v])
        .collect()
}

fn graph_segments(one: &OneBuff, log_start: i64, log_end: i64) -> Vec<(i64, i64, i64)> {
    let segs = item_segments(one, None);
    if segs.is_empty() {
        vec![(log_start, log_end, 0)]
    } else {
        stats::fuse_segments(log_start, log_end, segs)
    }
}

/// NPC（target）的 boons/conditions states —— P3a WvW 伪 target 无仿真 → 空。
pub fn npc_states(_log: &ParsedLog, _actor_sims: &ActorSims) -> (Vec<Vec<i64>>, Vec<Vec<i64>>) {
    (Vec::new(), Vec::new())
}

/// NPC target 的 map ids（fake target 无）。
pub fn npc_map_ids(_actor_sims: &ActorSims) -> BTreeSet<i64> {
    BTreeSet::new()
}
