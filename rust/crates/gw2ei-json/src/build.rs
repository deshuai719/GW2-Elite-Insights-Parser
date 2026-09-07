//! 顶层 JsonLog 组装(JsonLogBuilder.cs):标量 → targets → players → phases
//! → 四 map 收口(skillMap/buffMap/teamMap;damageModMap 本阶段空)。

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, Friend, ParsedLog};

use crate::actors::{self, Collectors};
use crate::content::BuffClassification;
use crate::ctx::{duration_string, format_unix_seconds, Ctx};
use crate::dto_a::*;
use crate::dto_b::*;
use crate::rows::{ActorRows, BuffApplyLine};
use crate::skills::SkillTable;

pub fn build_json(ctx: &Ctx) -> Result<JsonLog, crate::BuildError> {
    let log = ctx.log;
    let meta = &ctx.meta;
    let mut cols = Collectors::default();
    let skills = SkillTable::from_names(&log.skills, ctx.content);

    // ---- targets(NPC;WvW 伪 target)----
    let targets: Vec<JsonNpc> = log
        .targets
        .iter()
        .map(|t| build_npc(ctx, &skills, &mut cols, t.agent))
        .collect();
    // ---- players(玩家 + 友方非小队)----
    let players: Vec<JsonPlayer> = log
        .friendlies
        .iter()
        .map(|f| match f {
            Friend::Player(i) => build_player(ctx, &skills, &mut cols, log.players[*i].agent, false),
            Friend::NonSquad(i) => {
                let ns = &log.friendly_non_squad[*i];
                build_player(ctx, &skills, &mut cols, ns.agent, true)
            }
        })
        .collect();

    // ---- phases(单主 phase "Full Fight";JsonPhaseBuilder)----
    let ph = &log.log_data.main_phase;
    let target_priorities: std::collections::BTreeMap<String, String> = ph
        .targets
        .iter()
        .enumerate()
        .map(|(i, _)| (i.to_string(), "MAIN".to_string()))
        .collect();
    let phases = vec![JsonPhase {
        start: ph.start,
        end: ph.end,
        name: ph.name.clone(),
        targets: (0..ph.targets.len() as i64).collect(),
        secondary_targets: vec![],
        target_priorities,
        phase_type: "Encounter".to_string(),
        breakbar_phase: false,
        sub_phases: None,
        success: None,
        is_legendary_cm: None,
        is_cm: None,
        ei_encounter_id: None,
        encounter_icon: None,
        encounter_is_late_start: None,
        encounter_missing_pre_event: None,
        encounter_phase: None,
        breakbar_recovered: None,
        breakbar_active: None,
    }];

    // ---- 四 map 收口 ----
    let mut skill_descs = std::collections::BTreeMap::new();
    for &id in cols.skill_map.keys() {
        let d = build_skill_desc(ctx, &skills, id);
        skill_descs.insert(format!("s{id}"), d);
    }
    let mut buff_descs = std::collections::BTreeMap::new();
    for &id in cols.buff_map.keys() {
        if let Some(d) = build_buff_desc(ctx, id) {
            buff_descs.insert(format!("b{id}"), d);
        }
    }
    let mut team_desc = std::collections::BTreeMap::new();
    for &tid in &cols.team_map {
        if let Some(guid) = team_guid(ctx, tid) {
            team_desc.insert(format!("t{tid}"), TeamDesc { guid });
        }
    }

    // ---- 标量 ----
    let map_id = ctx.map_id;
    let (fight_name, icon) = crate::build::wvw_names(map_id);
    let (pov_name, pov_account) = pov_identity(log, meta);
    let (unix_start, unix_end) = unix_range(meta, log.log_data.log_end);
    let duration_ms = log.log_data.log_end - log.log_data.log_start;
    let is_cm = false;
    let gw2_build = meta.gw2_build;
    let region = meta
        .region
        .as_ref()
        .map(|r| r.json())
        .filter(|r| r != "Unknown");
    let language = meta.language.map(language_json).unwrap_or_else(|| "Unknown".to_string());
    let language_id = meta.language.map(language_id).unwrap_or(0);
    let log_errors = if meta.errors.is_empty() {
        None
    } else {
        Some(meta.errors.clone())
    };
    // C# LogMetadata.cs:110-112:instanceStartSeconds =
    // TimeOffsetFromInstanceCreation / 1000(整数除法),再 unixStart - 秒。
    let (instance_time_start_std, instance_ip) = match (meta.instance_time_offset, &meta.instance_ip) {
        (Some(off), ip) => (
            unix_start
                .map(|us| format_unix_seconds(us - (off / 1000) as f64, true))
                .or_else(|| ip.clone()),
            ip.clone(),
        ),
        (None, ip) => (None, ip.clone()),
    };

    Ok(JsonLog {
        parsing_settings: ParsingSettings {
            parse_extensions: ctx.opts.parse_extensions,
            compute_phases: ctx.opts.compute_phases,
            compute_combat_replay: ctx.opts.compute_combat_replay,
            compute_damage_modifiers: ctx.opts.compute_damage_modifiers,
            compute_damage: ctx.opts.compute_damage,
            compute_cast: ctx.opts.compute_cast,
            compute_buff: ctx.opts.compute_buff,
            compute_mechanics: ctx.opts.compute_mechanics,
        },
        elite_insights_version: "3.28.0.1".to_string(),
        trigger_id: i64::from(log.log_data.trigger_id),
        is_instance_log: false,
        ei_encounter_id: 459520,
        ei_log_id: 459520,
        map_id,
        fight_name: fight_name.clone(),
        name: fight_name.clone(),
        fight_icon: icon.clone(),
        icon,
        arc_version: format!("EVTC{}", log.arc_version.0),
        arc_revision: i64::from(log.arc_version.1),
        gw2_build: gw2_build as i64,
        language,
        fractal_scale: i64::from(meta.fractal_scale.unwrap_or(0)),
        region: region.unwrap_or_default(),
        language_id,
        recorded_by: pov_name,
        recorded_account_by: pov_account,
        time_start: unix_start.map(|s| format_unix_seconds(s, false)).unwrap_or_else(|| "MISSING".into()),
        time_end: unix_end.map(|s| format_unix_seconds(s, false)).unwrap_or_else(|| "MISSING".into()),
        time_start_std: unix_start.map(|s| format_unix_seconds(s, true)).unwrap_or_else(|| "MISSING".into()),
        time_end_std: unix_end.map(|s| format_unix_seconds(s, true)).unwrap_or_else(|| "MISSING".into()),
        duration: duration_string(duration_ms),
        duration_ms,
        log_start_offset: log.log_data.log_start,
        instance_time_start_std,
        instance_ip,
        instance_privacy: "Not Applicable".to_string(),
        targetless: false,
        success: true,
        is_cm,
        is_legendary_cm: false,
        is_late_start: false,
        missing_pre_event: false,
        anonymous: false,
        detailed_wvw: false,
        targets,
        players,
        phases,
        mechanics: None,
        upload_links: vec![String::new()],
        skill_map: skill_descs,
        buff_map: buff_descs,
        damage_mod_map: Default::default(),
        team_map: team_desc,
        personal_buffs: Default::default(),
        personal_damage_mods: Default::default(),
        log_errors,
        combat_replay_meta_data: None,
        wvw_map_data: None,
    })
}

/// WvW fightName/fightIcon(WvWLogic.GetLogicName;MapIDs 子集)。guild hall
/// 地图(GildedHollow 等)与非 WvW 地图列表按需扩展。
pub fn wvw_names(map_id: i64) -> (String, String) {
    let default = "World vs World";
    match map_id {
        38 => (
            format!("{default} - Eternal Battlegrounds"),
            "https://wiki.guildwars2.com/images/thumb/9/9c/Eternal_Battlegrounds_loading_screen.jpg/240px-Eternal_Battlegrounds_loading_screen.jpg".into(),
        ),
        95 => (
            format!("{default} - Green Alpine Borderlands"),
            "https://wiki.guildwars2.com/images/thumb/4/44/Green_Alpine_Borderlands_loading_screen.jpg/240px-Green_Alpine_Borderlands_loading_screen.jpg".into(),
        ),
        96 => (
            format!("{default} - Blue Alpine Borderlands"),
            "https://wiki.guildwars2.com/images/thumb/b/be/Blue_Borderlands_loading_screen.jpg/240px-Blue_Borderlands_loading_screen.jpg".into(),
        ),
        1099 => (
            format!("{default} - Red Desert Borderlands"),
            "https://wiki.guildwars2.com/images/thumb/9/99/Red_Desert_Borderlands_loading_screen.jpg/240px-Red_Desert_Borderlands_loading_screen.jpg".into(),
        ),
        1100 => (
            format!("{default} - Obsidian Sanctum"),
            "https://wiki.guildwars2.com/images/thumb/9/9c/Eternal_Battlegrounds_loading_screen.jpg/240px-Eternal_Battlegrounds_loading_screen.jpg".into(),
        ),
        968 => (
            format!("{default} - Edge of the Mists"),
            "https://wiki.guildwars2.com/images/thumb/6/62/Edge_of_the_Mists_loading_screen.jpg/240px-Edge_of_the_Mists_loading_screen.jpg".into(),
        ),
        1102 => (
            format!("{default} - Armistice Bastion"),
            "https://wiki.guildwars2.com/images/thumb/9/9c/Eternal_Battlegrounds_loading_screen.jpg/240px-Eternal_Battlegrounds_loading_screen.jpg".into(),
        ),
        _ => (default.to_string(), "https://wiki.guildwars2.com/images/d/db/PvP_Server_Browser_%28map_icon%29.png".into()),
    }
}

fn pov_identity(log: &ParsedLog, meta: &crate::ctx::MetaSig) -> (String, String) {
    let Some(addr) = meta.pov_addr else {
        return ("N/A".into(), "N/A".into());
    };
    let mut t = log.agents.clone();
    let time = meta.pov_time.unwrap_or(0);
    let id = t.resolve_agent(addr, time);
    let Some(a) = (id != gw2ei_model::NO_AGENT).then(|| log.agents.slot(id)).flatten() else {
        return ("N/A".into(), "N/A".into());
    };
    (
        a.character.clone(),
        a.account.clone().unwrap_or_else(|| "N/A".into()),
    )
}

fn unix_range(meta: &crate::ctx::MetaSig, log_end: i64) -> (Option<f64>, Option<f64>) {
    let start = meta.unix_start.map(f64::from);
    let end = meta.unix_end.map(f64::from);
    match (start, end) {
        (Some(s), Some(e)) => (Some(s), Some(e)),
        (Some(s), None) => {
            let dur = (log_end as f64 / 1000.0 * 1000.0).round() / 1000.0;
            (Some(s), Some(s + dur))
        }
        (None, Some(e)) => {
            let dur = (log_end as f64 / 1000.0 * 1000.0).round() / 1000.0;
            (Some(e - dur), Some(e))
        }
        (None, None) => (None, None),
    }
}

fn language_json(l: gw2ei_parse::Language) -> String {
    l.display_name().to_string()
}

fn language_id(l: gw2ei_parse::Language) -> i64 {
    match l {
        gw2ei_parse::Language::Chinese => 5,
        gw2ei_parse::Language::English => 0,
        gw2ei_parse::Language::French => 1,
        gw2ei_parse::Language::German => 2,
        gw2ei_parse::Language::Spanish => 3,
        gw2ei_parse::Language::Missing | gw2ei_parse::Language::Unknown => 0,
    }
}

fn team_guid(ctx: &Ctx, team_id: i64) -> Option<String> {
    // TeamGUIDEvent(按 team id;事件流 scan:GuidTeam 无 team id 关联 ——
    // P1 GuidTeam(content_id?) 无法直接映射 team id;团队 guid 通过
    // GetWvWTeamsEvent 关联的 Guid 事件链 —— P2b 无该表,先 None。
    let _ = (ctx, team_id);
    None
}

// ===== 玩家构建 =====

#[allow(clippy::too_many_lines)]
pub fn build_player(ctx: &Ctx, skills: &SkillTable, cols: &mut Collectors, agent: gw2ei_model::AgentId, non_squad: bool) -> JsonPlayer {
    let log = ctx.log;
    let a = log.agents.slot(agent).expect("player agent");
    let actor = ActorRows::new(log, &ctx.rows, agent);
    let b = actors::ActorBuilder { ctx, actor };
    let phase = &log.log_data.main_phase;
    let start = phase.start;
    let end = phase.end;
    let target_id = log.log_data.dummy_target_agent;
    let dur = (end - start) as f64 / 1000.0;

    // ---- dpsAll/dpsTargets ----
    let all_rows: Vec<gw2ei_model::DmgRow> = b
        .actor
        .health_out(start, end, None)
        .iter()
        .filter_map(|hl| {
            let r = crate::stats::row_view(ctx, hl)?;
            Some(r)
        })
        .collect();
    let self_rows: Vec<gw2ei_model::DmgRow> = all_rows
        .iter()
        .filter(|r| r.from != gw2ei_model::NO_AGENT && log.agents.englobing_root(r.from) == agent)
        .copied()
        .collect();
    let (bb, bb_self) = actor_breakbar(&b, start, end);
    let dps_all = crate::stats::build_dps(&all_rows, &self_rows, bb, bb_self, start, end);
    // target 过滤行
    let t_all: Vec<gw2ei_model::DmgRow> = b
        .actor
        .health_out(start, end, Some(target_id))
        .iter()
        .filter_map(|hl| crate::stats::row_view(ctx, hl))
        .collect();
    let t_self: Vec<gw2ei_model::DmgRow> = t_all
        .iter()
        .filter(|r| r.from != gw2ei_model::NO_AGENT && log.agents.englobing_root(r.from) == agent)
        .copied()
        .collect();
    let (bb_t, bb_t_self) = actor_breakbar_target(&b, start, end, target_id);
    let dps_targets = vec![crate::stats::build_dps(&t_all, &t_self, bb_t, bb_t_self, start, end)];

    // ---- statsAll/statsTargets ----
    let can_crit = |id: i64| crate::content::skill_can_crit(ctx.content, id, ctx.meta.gw2_build);
    let off_all = crate::stats::compute_offensive(&b.actor, ctx, start, end, None, &can_crit);
    let off_target = crate::stats::compute_offensive(&b.actor, ctx, start, end, Some(target_id), &can_crit);
    let gp = gameplay_of(ctx, agent, start, end, skills);
    let stats_all = JsonGameplayStatsAll {
        wasted: gp.interrupted_count,
        time_wasted: gp.interrupted_duration,
        saved: gp.after_cast_interrupted_count,
        time_saved: gp.after_cast_interrupted_duration,
        stack_dist: 0.0,
        dist_to_com: 0.0,
        avg_boons: 0.0,
        avg_active_boons: 0.0,
        avg_conditions: 0.0,
        avg_active_conditions: 0.0,
        swap_count: gp.swap_count,
        skill_cast_uptime: gp.cast_uptime,
        skill_cast_uptime_no_aa: gp.cast_uptime_no_aa,
        off: crate::stats::game_stats(&off_all),
    };
    let stats_targets = vec![crate::stats::game_stats(&off_target)];

    // ---- defenses / support ----
    let (down_count, vapor) = down_apply_rows(ctx, agent);
    let strip = strip_stats(ctx, agent, start, end);
    let def = crate::stats::compute_defense(
        &b.actor,
        ctx,
        start,
        end,
        !non_squad,
        a.spec == gw2ei_model::Spec::Elementalist || a.base_spec == gw2ei_model::Spec::Elementalist,
        down_count,
        &vapor,
        strip,
    );
    let defenses = vec![JsonDefensesAll {
        damage_taken: def.damage_taken,
        damage_taken_count: def.damage_taken_count,
        condition_damage_taken: def.condi_taken,
        condition_damage_taken_count: def.condi_taken_count,
        power_damage_taken: def.power_taken,
        power_damage_taken_count: def.power_taken_count,
        strike_damage_taken: def.strike_taken,
        strike_damage_taken_count: def.strike_taken_count,
        life_leech_damage_taken: def.leech_taken,
        life_leech_damage_taken_count: def.leech_taken_count,
        downed_damage_taken: def.downed_taken,
        downed_damage_taken_count: def.downed_taken_count,
        damage_barrier: def.damage_barrier,
        damage_barrier_count: def.damage_barrier_count,
        breakbar_damage_taken: def.bb_taken,
        breakbar_damage_taken_count: def.bb_taken_count,
        blocked_count: def.blocked,
        evaded_count: def.evaded,
        missed_count: def.missed,
        dodge_count: def.dodge_count,
        invulned_count: def.invulned,
        interrupted_count: def.interrupted,
        down_count: def.down_count,
        down_duration: def.down_duration,
        dead_count: def.dead_count,
        dead_duration: def.dead_duration,
        dc_count: def.dc_count,
        dc_duration: def.dc_duration,
        boon_strips: def.boon_strips,
        boon_strips_time: def.boon_strips_time,
        condition_cleanses: def.condition_cleanses,
        condition_cleanses_time: def.condition_cleanses_time,
        received_crowd_control: def.cc_received,
        received_crowd_control_duration: def.cc_received_duration,
        stun_break: def.stun_break,
        removed_stun_duration: def.removed_stun_duration,
    }];
    let support = support_of(ctx, agent, start, end, skills);
    let support = vec![support];

    // ---- dist 三件套 + target dist ----
    let total_damage_dist = vec![actors::build_damage_dist(&b, start, end, None, skills, cols, &can_crit)];
    let total_damage_taken = vec![actors::build_damage_taken_dist(&b, start, end, None, skills, cols)];
    let target_damage_dist = vec![vec![actors::build_damage_dist(&b, start, end, Some(target_id), skills, cols, &can_crit)]];

    // ---- 1s 图 ----
    let damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::All)];
    let power_damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::Power)];
    let condition_damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::Condition)];
    let damage_taken_1s = vec![b.dmg_taken_graph_1s(start, end, actors::DmgType::All)];
    let power_damage_taken_1s = vec![b.dmg_taken_graph_1s(start, end, actors::DmgType::Power)];
    let condition_damage_taken_1s = vec![b.dmg_taken_graph_1s(start, end, actors::DmgType::Condition)];
    let breakbar_damage_taken_1s = vec![None::<Vec<f64>>];
    let target_damage_1s = vec![vec![b.dmg_graph_1s(start, end, Some(target_id), actors::DmgType::All)]];
    let target_power_damage_1s = vec![vec![b.dmg_graph_1s(start, end, Some(target_id), actors::DmgType::Power)]];
    let target_condition_damage_1s = vec![vec![b.dmg_graph_1s(start, end, Some(target_id), actors::DmgType::Condition)]];

    // ---- 状态序列 ----
    let health_percents = b.percent_series(agent, actors::PercentKind::Health);
    let barrier_percents = b.percent_series(agent, actors::PercentKind::Barrier);
    let active_times = vec![b.active_duration(agent, start, end)];

    // ---- rotation / weapons ----
    let mut cast_rows = cast_rows_of(ctx, agent);
    // C# GetIntersectingCastEvents(JsonActorBuilder.cs:57)含武器切换事件
    // (WeaponSwapEvent 属 CastEvent)→ rotation 的 -2 组。
    cast_rows.extend(swap_rows_of(ctx, agent));
    cast_rows.sort_by_key(|&i| ctx.log.events[i].time().unwrap_or(0));
    let rotation = actors::build_rotation(&b, start, end, skills, cols, &cast_rows);
    let swap_rows = swap_rows_of(ctx, agent);
    let sets = actors::estimate_weapons(log, agent, !non_squad, ctx.opts.compute_cast, &cast_rows, &swap_rows, skills);
    let weapon_sets: Vec<JsonWeaponSet> = sets
        .iter()
        .map(|s| JsonWeaponSet {
            weapons: s.weapons.clone(),
            timeframe: vec![s.start, s.end],
        })
        .collect();
    let weapons = weapon_sets.last().map(|s| s.weapons.clone()).unwrap_or_default();

    // ---- consumables / death recap ----
    let consumables = consumables_of(ctx, agent);
    // ---- buff 出现收集(buffMap 键来源:BuffsUptime/Volumes/NPC buffs 的
    // 引用集 —— 对 actor 的 apply/remove 事件 id;JsonBuffsUptimeBuilder:50
    // 等收集点在数值块(OOS)但 map 键需对齐)----
    if let Some(rows) = ctx.aux.buff_apply_to.get(&agent) {
        for l in rows {
            cols.buff_map.insert(l.buff_id, ());
        }
    }
    if let Some(rows) = ctx.aux.buff_remove_all_by_to.get(&agent) {
        for l in rows {
            cols.buff_map.insert(l.buff_id, ());
        }
    }
    let mut last_death = 0i64;
    let dead_rows = ctx.aux.dead_rows.get(&agent).cloned().unwrap_or_default();
    let down_rows = ctx.aux.down_rows.get(&agent).cloned().unwrap_or_default();
    let up_rows = ctx.aux.up_rows.get(&agent).cloned().unwrap_or_default();
    let mut death_recap: Vec<JsonDeathRecap> = Vec::new();
    let deaths_sorted: Vec<usize> = {
        let mut v = dead_rows.clone();
        v.sort_by_key(|&e| log.events[e].time().unwrap_or(0));
        v
    };
    for &d in &deaths_sorted {
        let rec = death_recap_one(ctx, agent, d, &down_rows, &up_rows, &mut last_death);
        // C#:每 DeadEvent 一条(JsonDeathRecapBuilder;toDown/toKill 为空时
        // 子键省略但对象保留 deathTime)。
        death_recap.push(rec);
    }

    // ---- 身份 ----
    let pa = log.agents.slot(agent).expect("agent");
    let (name, account) = if non_squad {
        // PlayerNonSquad.cs:10-18 —— Character/Account 为合成名(model 层已
        // 生成;agent 原始名不经 JSON)。record 由调用方保证存在。
        let ns = log
            .friendly_non_squad
            .iter()
            .find(|n| n.agent == agent)
            .expect("non-squad record");
        (ns.character.clone(), ns.account.clone())
    } else {
        (
            log.players
                .iter()
                .find(|p| p.agent == agent)
                .map(|p| p.character.clone())
                .unwrap_or_else(|| pa.character.clone()),
            log.players
                .iter()
                .find(|p| p.agent == agent)
                .map(|p| p.account.clone())
                .unwrap_or_default(),
        )
    };
    let group = if non_squad {
        log.friendly_non_squad
            .iter()
            .find(|n| n.agent == agent)
            .map(|n| i64::from(n.group))
            .unwrap_or(51)
    } else {
        log.players
            .iter()
            .find(|p| p.agent == agent)
            .map(|p| i64::from(p.group))
            .unwrap_or(1)
    };
    let guild_id = b.guild_id(agent);
    let team_id = b.team_id(agent);
    if team_id > 0 {
        cols.team_map.insert(team_id);
    }
    let is_englobed = log.agents.englobing_root(agent) != agent;
    let _ = dur;

    JsonPlayer {
        account,
        group,
        has_commander_tag: false,
        profession: pa.spec.csharp_name().to_string(),
        friendly_npc: false,
        not_in_squad: non_squad,
        guild_id,
        is_englobed,
        weapons,
        weapon_sets,
        dps_targets: vec![dps_targets],
        damage_taken_1s,
        power_damage_taken_1s,
        condition_damage_taken_1s,
        breakbar_damage_taken_1s,
        target_damage_1s,
        target_power_damage_1s,
        target_condition_damage_1s,
        target_damage_dist,
        stats_targets: vec![stats_targets],
        support,
        active_times,
        name,
        first_aware: pa.first_aware,
        last_aware: pa.last_aware,
        total_health: b.total_health(agent),
        condition: i64::from(pa.condition),
        concentration: i64::from(pa.concentration),
        healing: i64::from(pa.healing),
        toughness: i64::from(pa.toughness),
        hitbox_height: i64::from(pa.hitbox_height),
        hitbox_width: i64::from(pa.hitbox_width),
        instance_id: i64::from(pa.instid),
        team_id,
        is_fake: pa.is_fake,
        dps_all: vec![dps_all],
        stats_all: vec![stats_all],
        defenses,
        total_damage_dist,
        total_damage_taken,
        rotation: rotation.filter(|r| !r.is_empty()),
        damage_1s,
        power_damage_1s,
        condition_damage_1s,
        health_percents,
        barrier_percents,
        consumables,
        death_recap: if death_recap.is_empty() { None } else { Some(death_recap) },
    }
}

pub fn build_npc(ctx: &Ctx, skills: &SkillTable, cols: &mut Collectors, agent: gw2ei_model::AgentId) -> JsonNpc {
    let log = ctx.log;
    let a = log.agents.slot(agent).expect("npc agent");
    let actor = ActorRows::new(log, &ctx.rows, agent);
    let b = actors::ActorBuilder { ctx, actor };
    let phase = &log.log_data.main_phase;
    let start = phase.start;
    let end = phase.end;
    let all_rows: Vec<gw2ei_model::DmgRow> = b
        .actor
        .health_out(start, end, None)
        .iter()
        .filter_map(|hl| crate::stats::row_view(ctx, hl))
        .collect();
    let self_rows: Vec<gw2ei_model::DmgRow> = all_rows
        .iter()
        .filter(|r| r.from != gw2ei_model::NO_AGENT && log.agents.englobing_root(r.from) == agent)
        .copied()
        .collect();
    let (bb, bb_self) = actor_breakbar(&b, start, end);
    let dps_all = crate::stats::build_dps(&all_rows, &self_rows, bb, bb_self, start, end);
    let can_crit = |id: i64| crate::content::skill_can_crit(ctx.content, id, ctx.meta.gw2_build);
    let off_all = crate::stats::compute_offensive(&b.actor, ctx, start, end, None, &can_crit);
    let gp = gameplay_of(ctx, agent, start, end, skills);
    let stats_all = JsonGameplayStatsAll {
        wasted: gp.interrupted_count,
        time_wasted: gp.interrupted_duration,
        saved: gp.after_cast_interrupted_count,
        time_saved: gp.after_cast_interrupted_duration,
        stack_dist: 0.0,
        dist_to_com: 0.0,
        avg_boons: 0.0,
        avg_active_boons: 0.0,
        avg_conditions: 0.0,
        avg_active_conditions: 0.0,
        swap_count: gp.swap_count,
        skill_cast_uptime: gp.cast_uptime,
        skill_cast_uptime_no_aa: gp.cast_uptime_no_aa,
        off: crate::stats::game_stats(&off_all),
    };
    let (down_count, vapor) = down_apply_rows(ctx, agent);
    let strip = strip_stats(ctx, agent, start, end);
    let def = crate::stats::compute_defense(
        &b.actor, ctx, start, end, false, false, down_count, &vapor, strip,
    );
    let defenses = vec![JsonDefensesAll {
        damage_taken: def.damage_taken,
        damage_taken_count: def.damage_taken_count,
        condition_damage_taken: def.condi_taken,
        condition_damage_taken_count: def.condi_taken_count,
        power_damage_taken: def.power_taken,
        power_damage_taken_count: def.power_taken_count,
        strike_damage_taken: def.strike_taken,
        strike_damage_taken_count: def.strike_taken_count,
        life_leech_damage_taken: def.leech_taken,
        life_leech_damage_taken_count: def.leech_taken_count,
        downed_damage_taken: def.downed_taken,
        downed_damage_taken_count: def.downed_taken_count,
        damage_barrier: def.damage_barrier,
        damage_barrier_count: def.damage_barrier_count,
        breakbar_damage_taken: def.bb_taken,
        breakbar_damage_taken_count: def.bb_taken_count,
        blocked_count: def.blocked,
        evaded_count: def.evaded,
        missed_count: def.missed,
        dodge_count: def.dodge_count,
        invulned_count: def.invulned,
        interrupted_count: def.interrupted,
        down_count: def.down_count,
        down_duration: def.down_duration,
        dead_count: def.dead_count,
        dead_duration: def.dead_duration,
        dc_count: def.dc_count,
        dc_duration: def.dc_duration,
        boon_strips: def.boon_strips,
        boon_strips_time: def.boon_strips_time,
        condition_cleanses: def.condition_cleanses,
        condition_cleanses_time: def.condition_cleanses_time,
        received_crowd_control: def.cc_received,
        received_crowd_control_duration: def.cc_received_duration,
        stun_break: def.stun_break,
        removed_stun_duration: def.removed_stun_duration,
    }];
    let total_damage_dist = vec![actors::build_damage_dist(&b, start, end, None, skills, cols, &can_crit)];
    let total_damage_taken = vec![actors::build_damage_taken_dist(&b, start, end, None, skills, cols)];
    let damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::All)];
    let power_damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::Power)];
    let condition_damage_1s = vec![b.dmg_graph_1s(start, end, None, actors::DmgType::Condition)];
    let health_percents = b.percent_series(agent, actors::PercentKind::Health);
    let barrier_percents = b.percent_series(agent, actors::PercentKind::Barrier);
    let cast_rows = cast_rows_of(ctx, agent);
    let rotation = actors::build_rotation(&b, start, end, skills, cols, &cast_rows);
    // NPC 特有(伪 target)
    let hp_left = if true { 0.0 } else { 100.0 }; // encounter phase success
    let health_percent_burned = 100.0 - hp_left;
    let total_health = b.total_health(agent);
    let breakbar_percents: Vec<Vec<f64>> = Vec::new();
    JsonNpc {
        id: i64::from(a.id),
        final_health: -1,
        final_barrier: -1,
        barrier_percent: 0.0,
        health_percent_burned,
        enemy_player: false,
        breakbar_percents,
        name: a.character.clone(),
        first_aware: a.first_aware,
        last_aware: a.last_aware,
        total_health,
        condition: i64::from(a.condition),
        concentration: i64::from(a.concentration),
        healing: i64::from(a.healing),
        toughness: i64::from(a.toughness),
        hitbox_height: i64::from(a.hitbox_height),
        hitbox_width: i64::from(a.hitbox_width),
        instance_id: i64::from(a.instid),
        team_id: b.team_id(agent),
        is_fake: a.is_fake,
        dps_all: vec![dps_all],
        stats_all: vec![stats_all],
        defenses,
        total_damage_dist,
        total_damage_taken,
        rotation: rotation.filter(|r| !r.is_empty()),
        damage_1s,
        power_damage_1s,
        condition_damage_1s,
        health_percents,
        barrier_percents,
    }
}

// ===== 辅助聚合 =====

fn actor_breakbar(b: &actors::ActorBuilder, start: i64, end: i64) -> (f64, f64) {
    let mut total = 0.0f64;
    let mut selfs = 0.0f64;
    for l in b.actor.breakbar_out(start, end) {
        let v = b.actor.breakbar_value(&l);
        total += v;
        if l.from == b.actor.root {
            selfs += v;
        }
    }
    (total, selfs)
}

fn actor_breakbar_target(b: &actors::ActorBuilder, start: i64, end: i64, target: gw2ei_model::AgentId) -> (f64, f64) {
    let log = b.ctx.log;
    let mut total = 0.0f64;
    let mut selfs = 0.0f64;
    for l in b.actor.breakbar_out(start, end) {
        if log.agents.englobing_root(l.to) != log.agents.englobing_root(target) {
            continue;
        }
        let v = b.actor.breakbar_value(&l);
        total += v;
        if l.from == b.actor.root {
            selfs += v;
        }
    }
    (total, selfs)
}

fn gameplay_of(ctx: &Ctx, agent: gw2ei_model::AgentId, start: i64, end: i64, skills: &SkillTable) -> crate::stats::GameplayAcc {
    let log = ctx.log;
    let mut root = agent;
    if log.agents.englobing_root(agent) != gw2ei_model::NO_AGENT {
        root = log.agents.englobing_root(agent);
    }
    // C# GetCastEvents = 动画 cast + 武器切换事件(C# WeaponSwapEvent 属
    // CastEvent;GameplayStatistics 的 WeaponSwapCount 计 switch 事件)。
    let mut cast_rows = cast_rows_of(ctx, root);
    cast_rows.extend(swap_rows_of(ctx, root));
    cast_rows.sort_by_key(|&i| ctx.log.events[i].time().unwrap_or(0));
    let is_aa = |id: i64| {
        skills
            .get(id)
            .map(|s| s.api_aa)
            .unwrap_or(false)
            || crate::skills::is_aa_override(id, ctx.meta.gw2_build)
    };
    crate::stats::compute_gameplay(log, root, start, end, &cast_rows, &is_aa)
}

fn cast_rows_of(ctx: &Ctx, agent: gw2ei_model::AgentId) -> Vec<usize> {
    let mut v = ctx.aux.cast_by_caster.get(&agent).cloned().unwrap_or_default();
    if let Some(a) = ctx.log.agents.slot(agent) {
        for &e in &a.englobed {
            if let Some(rows) = ctx.aux.cast_by_caster.get(&e) {
                v.extend(rows.iter().copied());
            }
        }
    }
    v.sort_by_key(|&i| ctx.log.events[i].time().unwrap_or(0));
    v
}

fn swap_rows_of(ctx: &Ctx, agent: gw2ei_model::AgentId) -> Vec<usize> {
    let mut v = ctx.aux.weapon_swap_by_caster.get(&agent).cloned().unwrap_or_default();
    if let Some(a) = ctx.log.agents.slot(agent) {
        for &e in &a.englobed {
            if let Some(rows) = ctx.aux.weapon_swap_by_caster.get(&e) {
                v.extend(rows.iter().copied());
            }
        }
    }
    v.sort_by_key(|&i| ctx.log.events[i].time().unwrap_or(0));
    v
}

/// 玩家 DownCount:Downed(770)BuffApply(by to)。
fn down_apply_rows(ctx: &Ctx, agent: gw2ei_model::AgentId) -> (i64, Vec<i64>) {
    let mut count = 0i64;
    let mut vapor: Vec<i64> = Vec::new();
    let rows = ctx.aux.buff_apply_to.get(&agent).cloned().unwrap_or_default();
    let mut downs: Vec<i64> = Vec::new();
    for l in rows {
        if l.buff_id == 770 {
            downs.push(l.time);
        }
    }
    // VaporForm remove all(±10ms 内)
    let removes = ctx.aux.buff_remove_all_by_to.get(&agent).cloned().unwrap_or_default();
    let vapor_removes: Vec<i64> = removes.iter().filter(|l| l.buff_id == 5620).map(|l| l.time).collect();
    for t in downs {
        if !vapor_removes.iter().any(|vt| (vt - t).abs() < 10) {
            count += 1;
        }
    }
    let _ = &mut vapor;
    (count, vapor_removes)
}

/// GetStripData(DefensePerTargetStatistics.cs:48-70):Boon 分类 → 敌方 strip;
/// Condition 分类 → 已方 cleanse。判定对象 = **被移除者 actor**(to==actor
/// 的 RemoveAll 行):boon 剔除 by unknown 与 by==actor;condi 仅剔除
/// by unknown(C# excludeSelf 参数不同)。
fn strip_stats(ctx: &Ctx, agent: gw2ei_model::AgentId, start: i64, end: i64) -> (i64, f64, i64, f64) {
    let log = ctx.log;
    let mut boon_strip = 0i64;
    let mut boon_time = 0.0f64;
    let mut condi_cleanse = 0i64;
    let mut condi_time = 0.0f64;
    let remove_rows = ctx.aux.buff_remove_all_by_to.get(&agent).cloned().unwrap_or_default();
    let rows_by_id: std::collections::BTreeMap<i64, Vec<&crate::rows::BuffRemoveAllLine>> =
        remove_rows
            .iter()
            .filter(|l| l.kind == 0 && start <= l.time && l.time <= end)
            .fold(Default::default(), |mut m, l| {
                m.entry(l.buff_id).or_default().push(l);
                m
            });
    let actor_root = log.agents.englobing_root(agent);
    let log_dur = (end - start) as f64;
    for (id, rows) in rows_by_id {
        let Some(def) = ctx.registry.get(id) else { continue };
        // C# GetStripData:cap 是 **per-buff 组内**(Math.Min(组累计,
        // LogDuration)),跨 buff 相加可超上限。
        match def.classification {
            BuffClassification::Boon => {
                let mut cur = 0.0f64;
                let mut n = 0i64;
                for l in rows {
                    // excludeSelf=true:by unknown 或 by==actor 剔除
                    let by_unknown = l.by == gw2ei_model::NO_AGENT;
                    let by_self = !by_unknown && log.agents.englobing_root(l.by) == actor_root;
                    if by_unknown || by_self {
                        continue;
                    }
                    cur = (cur + f64::from(l.removed_duration)).min(log_dur);
                    n += 1;
                }
                boon_strip += n;
                boon_time += cur;
            }
            BuffClassification::Condition => {
                let mut cur = 0.0f64;
                let mut n = 0i64;
                for l in rows {
                    // excludeSelf=false:仅剔除 by unknown
                    if l.by == gw2ei_model::NO_AGENT {
                        continue;
                    }
                    cur = (cur + f64::from(l.removed_duration)).min(log_dur);
                    n += 1;
                }
                condi_cleanse += n;
                condi_time += cur;
            }
            _ => {}
        }
    }
    (
        boon_strip,
        (boon_time / 1000.0 * 1000.0).round_ties_even() / 1000.0,
        condi_cleanse,
        (condi_time / 1000.0 * 1000.0).round_ties_even() / 1000.0,
    )
}

fn support_of(ctx: &Ctx, agent: gw2ei_model::AgentId, start: i64, end: i64, skills: &SkillTable) -> JsonPlayerSupport {
    let log = ctx.log;
    // resurrect 数 = cast events 中 Resurrect(1066/1006)
    let cast_rows = cast_rows_of(ctx, agent);
    let mut res_count = 0i64;
    let mut res_dur = 0i64;
    for &i in &cast_rows {
        let evt = &log.events[i];
        let (time, actual, sid) = match evt {
            CombatEvent::AnimatedCast(f) => (f.time, f.actual_duration, f.skill_id),
            CombatEvent::Emote(f) => (f.base.time, f.base.actual_duration, f.base.skill_id),
            _ => continue,
        };
        if time < start || time > end {
            continue;
        }
        if crate::signed_id(sid) == 1066 || crate::signed_id(sid) == 1006 {
            res_count += 1;
            res_dur += i64::from(actual);
        }
    }
    // stun break(support 侧 = C# SupportAllStatistics 的
    // GetStunBreakData(actor):行 dst(施放来源)桶 —— arc 的 break-stun 行
    // dst=0,事件化方向修正(To=From)后该侧信息不保留 → 恒 0。received
    // 侧(defense/support-self 无)在 compute_defense 按行 src 桶统计。)
    let sb_count = 0i64;
    let sb_dur = 0.0f64;
    let sb_self = 0i64;
    let sb_self_dur = 0.0f64;
    // ---- strip/cleanse(SupportPerAllyStatistics + SupportStatistics 语义;
    // 全部按 by==actor 的 RemoveAll 行,to 侧分流)----
    let by_rows: Vec<&crate::rows::BuffRemoveAllLine> = ctx
        .aux
        .buff_remove_all_by_from
        .get(&agent)
        .map(|v| v.iter().filter(|l| l.kind == 0 && start <= l.time && l.time <= end).collect())
        .unwrap_or_default();
    let actor_root = log.agents.englobing_root(agent);
    // 其余玩家身份(SupportStatistics 的 PlayerList 循环;self 单独按
    // to==actor)。
    let player_agents: Vec<AgentId> = log.players.iter().map(|p| p.agent).collect();
    let mut foe_boons = 0i64;
    let mut foe_boon_dur = 0.0f64;
    let mut foe_dc = 0i64;
    let mut foe_dc_dur = 0.0f64;
    let mut self_cleanse = 0i64;
    let mut self_cleanse_dur = 0.0f64;
    let mut ally_cleanse = 0i64;
    let mut ally_cleanse_dur = 0.0f64;
    let log_dur = (end - start) as f64;
    // SupportPerAllyStatistics 的 cap 组键:
    // - foe removals(totals = to==null 的单份统计)→ per-buff(跨 to);
    // - friendly removals(self/其他玩家各一份)→ per-(to, buff)。
    let mut foe_by_buff: std::collections::BTreeMap<i64, Vec<&crate::rows::BuffRemoveAllLine>> =
        Default::default();
    let mut fr_by_key: std::collections::BTreeMap<(AgentId, i64), Vec<&crate::rows::BuffRemoveAllLine>> =
        Default::default();
    for l in by_rows {
        let Some(def) = ctx.registry.get(l.buff_id) else { continue };
        let is_boon = def.classification == BuffClassification::Boon;
        let is_condi = def.classification == BuffClassification::Condition;
        if !is_boon && !is_condi {
            continue;
        }
        if is_boon {
            // BoonStrip = totals.Foe + totals.Unknown(SupportStatistics.cs)
            if l.to_friendly {
                continue;
            }
            foe_boons += 1;
            foe_by_buff.entry(l.buff_id).or_default().push(l);
        } else if l.to_friendly {
            let key = (log.agents.englobing_root(l.to), l.buff_id);
            fr_by_key.entry(key).or_default().push(l);
        }
    }
    for (id, rows) in foe_by_buff {
        let mut foe_dur = 0.0f64;
        let mut foe_dc_cur = 0.0f64;
        for l in rows {
            foe_dur = (foe_dur + f64::from(l.removed_duration)).min(log_dur);
            // FoeRemovalsDownContribution:to.IsDownedBeforeNext90
            if ctx
                .downs
                .get(l.to)
                .is_some_and(|dc| dc.is_down_before_next_90(l.to, l.time))
            {
                foe_dc += 1;
                foe_dc_cur = (foe_dc_cur + f64::from(l.removed_duration)).min(log_dur);
            }
        }
        let _ = id;
        foe_boon_dur += foe_dur;
        foe_dc_dur += foe_dc_cur;
    }
    for ((to_root, _id), rows) in fr_by_key {
        let mut self_dur = 0.0f64;
        let mut ally_dur = 0.0f64;
        for l in rows {
            // Condition:FriendlyRemovals —— 按 to 归属(self / 其它玩家)
            if to_root == actor_root {
                self_cleanse += 1;
                self_dur = (self_dur + f64::from(l.removed_duration)).min(log_dur);
            } else if player_agents.iter().any(|&p| log.agents.englobing_root(p) == to_root) {
                ally_cleanse += 1;
                ally_dur = (ally_dur + f64::from(l.removed_duration)).min(log_dur);
            }
        }
        self_cleanse_dur += self_dur;
        ally_cleanse_dur += ally_dur;
    }
    let r3 = |x: f64| (x / 1000.0 * 1000.0).round_ties_even() / 1000.0;
    let _ = &skills;
    JsonPlayerSupport {
        resurrects: res_count,
        resurrect_time: (res_dur as f64 / 1000.0 * 1000.0).round_ties_even() / 1000.0,
        condi_cleanse: ally_cleanse,
        condi_cleanse_time: r3(ally_cleanse_dur),
        condi_cleanse_self: self_cleanse,
        condi_cleanse_time_self: r3(self_cleanse_dur),
        boon_strips: foe_boons,
        boon_strips_time: r3(foe_boon_dur),
        boon_strip_down_contribution: foe_dc,
        boon_strip_down_contribution_time: r3(foe_dc_dur),
        stun_break: sb_count,
        removed_stun_duration: (sb_dur / 1000.0 * 1000.0).round_ties_even() / 1000.0,
        stun_break_self: sb_self,
        removed_stun_self_duration: (sb_self_dur / 1000.0 * 1000.0).round_ties_even() / 1000.0,
    }
}

/// consumables(Nourishment/Enhancement/OtherConsumable 分类的 apply)。
/// 收集序按注册表分类序(SingleActorBuffsHelper.cs:1094-1130:三分类注册
/// 表序遍历 + Sort(Time) —— 同 time 时稳定保留注册表序)。
fn consumables_of(ctx: &Ctx, agent: gw2ei_model::AgentId) -> Option<Vec<JsonConsumable>> {
    let applies: Vec<&BuffApplyLine> = ctx
        .aux
        .buff_apply_to
        .get(&agent)
        .map(|v| v.iter().collect())
        .unwrap_or_default();
    let mut items: Vec<JsonConsumable> = Vec::new();
    // C# `SetConsumablesList`:BuffsByClassification[Nourishment/Enhancement/
    // OtherConsumable] 三组按序拼接(组内保持注册表序;动态合成 buff 在
    // 注册表尾部)。
    for cls in [
        BuffClassification::Nourishment,
        BuffClassification::Enhancement,
        BuffClassification::OtherConsumable,
    ] {
        for b in ctx.registry.active.iter().filter(|b| b.classification == cls) {
            for l in applies.iter().filter(|l| l.buff_id == b.id) {
                if l.time <= ctx.log_end {
                    if let Some(ex) = items.iter_mut().find(|x| {
                        x.id == l.buff_id && (x.time - l.time).abs() < 10
                    }) {
                        ex.stack += 1;
                    } else {
                        items.push(JsonConsumable {
                            stack: 1,
                            duration: i64::from(l.applied_duration),
                            time: l.time,
                            id: l.buff_id,
                        });
                    }
                }
            }
        }
    }
    items.sort_by_key(|x| x.time);
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn death_recap_one(
    ctx: &Ctx,
    agent: gw2ei_model::AgentId,
    dead_ev: usize,
    down_rows: &[usize],
    up_rows: &[usize],
    last_death: &mut i64,
) -> JsonDeathRecap {
    let log = ctx.log;
    let dead_time = log.events[dead_ev].time().unwrap_or(0);
    let last = *last_death;
    *last_death = dead_time;
    let taken = ctx
        .rows
        .health_by_to
        .get(&agent)
        .cloned()
        .unwrap_or_default();
    let downed = {
        let upped = up_rows
            .iter()
            .filter_map(|&e| {
                let t = log.events[e].time().unwrap_or(0);
                (t <= dead_time && t >= last).then_some(t)
            })
            .next_back();
        match upped {
            Some(up_t) => down_rows
                .iter()
                .filter_map(|&e| {
                    let t = log.events[e].time().unwrap_or(0);
                    (t <= dead_time && t >= up_t).then_some(t)
                })
                .next_back(),
            None => down_rows
                .iter()
                .filter_map(|&e| {
                    let t = log.events[e].time().unwrap_or(0);
                    (t <= dead_time && t >= last).then_some(t)
                })
                .next_back(),
        }
    };
    let window = |a: i64, b: i64, kill: bool| -> Vec<&crate::rows::HealthLine> {
        taken
            .iter()
            .filter(|hl| {
                let Some(row) = crate::stats::row_view(ctx, hl) else { return false };
                hl.line.time > a
                    && hl.line.time <= b
                    && (row.has_hit || if kill { row.has_killed } else { row.has_downed })
            })
            .collect()
    };
    let mk_items = |rows: Vec<&crate::rows::HealthLine>, cap: bool| -> Vec<JsonDeathRecapItem> {
        let mut sorted = rows;
        sorted.sort_by_key(|hl| hl.line.time);
        let mut out = Vec::new();
        let mut damage = 0i64;
        for hl in sorted.iter().rev() {
            out.push(recap_item_from(ctx, agent, hl));
            damage += recap_dmg(ctx, hl);
            if cap && damage > 20_000 {
                break;
            }
        }
        out
    };
    let mut recap = JsonDeathRecap { death_time: dead_time, to_down: None, to_kill: None };
    if let Some(down_t) = downed {
        let d2d = mk_items(window(last, down_t, false), true);
        if !d2d.is_empty() {
            recap.to_down = Some(d2d);
        }
        let d2k = mk_items(window(down_t, dead_time, true), false);
        if !d2k.is_empty() {
            recap.to_kill = Some(d2k);
        }
    } else {
        let d2k = mk_items(window(last, dead_time, true), true);
        if !d2k.is_empty() {
            recap.to_kill = Some(d2k);
        }
    }
    recap
}

fn recap_dmg(ctx: &Ctx, hl: &crate::rows::HealthLine) -> i64 {
    match &ctx.log.events[hl.line.ev] {
        CombatEvent::DirectHealthDamage(f)
        | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => i64::from(f.health_damage),
        _ => 0,
    }
}

fn recap_item_from(ctx: &Ctx, agent: gw2ei_model::AgentId, hl: &crate::rows::HealthLine) -> JsonDeathRecapItem {
    let log = ctx.log;
    let (id, indirect, src_addr, damage, time) = match &log.events[hl.line.ev] {
        CombatEvent::DirectHealthDamage(f) | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => (
            i64::from(f.skill.skill_id),
            matches!(&log.events[hl.line.ev], CombatEvent::NonDirectHealthDamage(_)),
            f.skill.from,
            i64::from(f.health_damage),
            f.skill.time,
        ),
        _ => (0, false, 0, 0, 0),
    };
    let mut resolver = log.agents.clone();
    let from_id = if src_addr != 0 {
        resolver.resolve_agent(src_addr, time)
    } else {
        gw2ei_model::NO_AGENT
    };
    let char = if from_id != gw2ei_model::NO_AGENT && from_id != agent {
        log.agents.slot(from_id).map(|a| a.character.clone()).unwrap_or_default()
    } else {
        // C# 行 from 为补缺 UNKNOWN agent / 环境来源 → 名 "UNKNOWN"
        // (不是被击杀者名)。
        "UNKNOWN".to_string()
    };
    JsonDeathRecapItem { id, indirect_damage: indirect, src: char, damage, time }
}

// ===== skill/buff desc(JsonLogBuilder)=====

fn build_skill_desc(ctx: &Ctx, skills: &SkillTable, id: i64) -> SkillDesc {
    let skill = skills.get(id);
    let mut name = skill.map(|s| s.name.clone()).unwrap_or_else(|| id.to_string());
    let mut icon = skill
        .map(|s| s.icon.clone())
        .unwrap_or_else(|| crate::content::default_skill_icon().to_string());
    // 表外条目(合成/负 id 技能):overrides 名字/图标优先 —— 与
    // SkillItem 构造链一致(C# SkillData.Get 对未知 id 建占位,名取自
    // evtc/override;如 Weapon Swap=-2)。
    if skill.is_none() {
        if let Some(o) = ctx.content.overrides.names.get(&id) {
            name = o.clone();
        }
        if let Some(o) = ctx.content.overrides.icons.get(&id) {
            icon = o.clone();
        }
    }
    // Buff 覆盖(BuffsContainer.cs:151-160 OverrideFromBuff + SkillItem.cs:69
    // CanOverrideFromBuffs):注册表同 id buff 恒覆盖 icon;name 仅在
    // UNKNOWN / 全数字 / override 名单内时覆盖。
    if let Some(def) = ctx.registry.get(id) {
        icon = def.icon.clone();
        let unknown = name == "UNKNOWN";
        let all_digit = !name.is_empty() && name.chars().all(|c| c.is_ascii_digit());
        let overridden = ctx.content.overrides.names.contains_key(&id);
        if unknown || all_digit || overridden {
            name = def.name.clone();
        }
    }
    // 合成/未知技能:名回退 "UNKNOWN"(SkillItem.DefaultName;负 id 合成的
    // 技能无表项时以 id 数字为名 —— 与 C# SkillData.Get 一致)。
    let auto_attack = skill.map(|s| s.api_aa).unwrap_or(false)
        || crate::skills::is_aa_override(id, ctx.meta.gw2_build);
    SkillDesc {
        name: if name.is_empty() { "UNKNOWN".to_string() } else { name },
        auto_attack,
        can_crit: crate::content::skill_can_crit(ctx.content, id, ctx.meta.gw2_build),
        icon,
        is_swap: crate::skills::is_swap(id),
        is_instant_cast: false,
        is_trait_proc: false,
        is_unconditional_proc: false,
        is_gear_proc: false,
        is_not_accurate: false,
        conversion_based_healing: false,
        hybrid_healing: false,
    }
}

fn build_buff_desc(ctx: &Ctx, id: i64) -> Option<BuffDesc> {
    let def = ctx.registry.get(id)?;
    let classification = def.classification.json_name();
    Some(BuffDesc {
        name: def.name.clone(),
        classification: classification.map(String::from),
        icon: def.icon.clone(),
        stacking: def.is_intensity(),
        conversion_based_healing: false,
        hybrid_healing: false,
        descriptions: None,
    })
}
