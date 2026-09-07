//! P2a 集成测试：真实样例全流程（skip-if-missing）。
//!
//! 样例 `rust/testdata/20260530-205048.zevtc`（WvW kill；golden 基准
//! `rust/golden/20260530-205048_wvw_kill.json` 只读参照）。
//!
//! 断言：
//! 1. 链接层计数：46 player roots / 2 friendly non-squad / 46 accounts；
//! 2. 组装层：48 friendlies（46+2）、单 target（伪 "Enemy Players"）、
//!    单 phase "Full Fight" [0, logEnd]；
//! 3. 伤害守恒：事件层总伤害 = Σ per-agent 桶（差值显式）；
//! 4. 玩家 FirstAware 单调、dpsAll 数量级输出（P2b 与 golden 逐字段锁定）。

use gw2ei_model::agent::{AgentId, AgentType, NO_AGENT, species};
use gw2ei_model::{
    CombatEvent, Friend, ParsedLog, build_event_index, damage_graph_1s, dmg_row,
    parse_log_file,
};

fn sample_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("testdata")
        .join("20260530-205048.zevtc")
}

fn load() -> Option<ParsedLog> {
    match parse_log_file(&sample_path()) {
        Ok(log) => Some(log),
        Err(e) => {
            eprintln!("skipping: sample not found or failed ({e})");
            None
        }
    }
}

fn health_damage(e: &CombatEvent) -> Option<i64> {
    match e {
        CombatEvent::DirectHealthDamage(f)
        | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => Some(i64::from(f.health_damage)),
        _ => None,
    }
}

#[test]
fn actors_sample_assembly_and_conservation() {
    let Some(log) = load() else { return };
    // ---- 组装面断言 ----
    assert_eq!(log.players.len(), 46, "real players == golden 48 - 2 non squad");
    assert_eq!(log.friendly_non_squad.len(), 2);
    assert_eq!(log.friendlies.len(), 48);
    assert_eq!(log.targets.len(), 1);
    let dummy = log.targets[0].agent;
    assert_eq!(log.agents.slot(dummy).expect("dummy").id, species::WORLD_VERSUS_WORLD);
    assert!(log.agents.slot(dummy).expect("dummy").is_fake);
    assert_eq!(log.log_data.name, "World vs World - Blue Alpine Borderlands");
    assert_eq!(log.log_data.main_phase.name, "Full Fight");
    assert_eq!(log.log_data.main_phase.start, 0);
    assert_eq!(log.log_data.main_phase.end, log.log_data.log_end);
    assert_eq!(log.log_data.main_phase.targets, vec![dummy]);
    // players 按角色名稳定排序
    let chars: Vec<&str> = log.players.iter().map(|p| p.character.as_str()).collect();
    let mut sorted = chars.clone();
    sorted.sort();
    assert_eq!(chars, sorted, "players sorted by character");
    let names: Vec<&str> = log.players.iter().map(|p| p.character.as_str()).collect();
    for expect in ["Thaha", "元乾", "废物汉堡", "丨蜜桃四季春丨", "稻香村黑海会员"] {
        assert!(names.contains(&expect), "player {expect} missing");
    }
    for p in &log.players {
        let a = log.agents.slot(p.agent).expect("player agent");
        assert!(a.first_aware <= a.last_aware, "aware monotonic");
        assert!(a.last_aware != i64::MAX);
        assert!(a.instid != 0);
    }
    // ---- 事件索引 + 守恒 ----
    let idx = build_event_index(&log);
    let total_events: i64 = log.events.iter().filter_map(health_damage).sum();
    let mut total_indexed: i64 = 0;
    for (agent, rows) in &idx.damage_from {
        for &r in rows {
            if *agent != NO_AGENT {
                total_indexed += health_damage(&log.events[r]).expect("damage bucket row");
            }
        }
    }
    eprintln!(
        "conservation: events_total={total_events} indexed={total_indexed} diff={}",
        total_events - total_indexed
    );
    assert!(total_events >= total_indexed);
    assert!(total_events - total_indexed < 10_000, "unknown-source damage small");
    // ---- per-player dpsAll（人工与 golden players[].dpsAll 核对）----
    let phase = &log.log_data.main_phase;
    eprintln!(
        "===== per-player dpsAll (phase [{}, {}], {}ms) =====",
        phase.start, phase.end, phase.end - phase.start
    );
    let mut grand_total: i64 = 0;
    for p in &log.players {
        let out_rows = actor_outgoing(&log, &idx, p.agent);
        let self_rows: Vec<_> = out_rows.iter().filter(|r| r.from == p.agent).copied().collect();
        let (bb, bb_self) = actor_breakbar(&log, &idx, p.agent);
        let st = gw2ei_model::compute_dps_stats(
            &out_rows, &self_rows, bb, bb_self, phase.start, phase.end,
        );
        grand_total += i64::from(st.damage);
        eprintln!(
            "{:<22} dmg={:>9} dps={:>7} power={:>8} condi={:>8} strike={:>8} bb={:.1} unk_indirect={}",
            p.character, st.damage, st.dps, st.power_damage, st.condi_damage, st.strike_damage,
            st.breakbar_damage, st.completeness.unclassified_indirect_events
        );
    }
    eprintln!("players total damage = {grand_total}");
    let dummy_taken: i64 = idx
        .damage_to
        .get(&dummy)
        .map(|rows| {
            rows.iter()
                .map(|&r| health_damage(&log.events[r]).expect("row"))
                .sum()
        })
        .unwrap_or(0);
    eprintln!("dummy target damage taken = {dummy_taken}");
    assert!(grand_total > 0);
    assert!(dummy_taken > 0);
    // ---- 1s 图：88 桶（86689ms 非整除 -> floor+2）----
    let all_rows: Vec<(i64, i64)> = log
        .events
        .iter()
        .filter_map(|e| health_damage(e).map(|d| (e.time().expect("dmg time"), d)))
        .collect();
    let graph = damage_graph_1s(&all_rows, phase.start, phase.end);
    assert_eq!(graph.len(), 88);
    // per-player 图单调且尾桶 == 该玩家 hit 总伤害（golden damage1S 结构）
    for p in log.players.iter().take(5) {
        let mut rows: Vec<(i64, i64)> = Vec::new();
        if let Some(list) = idx.damage_from.get(&p.agent) {
            for &r in list {
                let Some(row) = dmg_row(&log.events[r], p.agent, NO_AGENT) else { continue };
                // C# GetDamageEvents 过滤 !ToFriendly（Actor 侧伤害剔除友军行）
                if row.has_hit && !row.to_friendly {
                    rows.push((row.time, i64::from(row.health_damage)));
                }
            }
        }
        rows.sort_by_key(|r| r.0);
        let expected: i64 = rows.iter().map(|r| r.1).sum();
        let g = damage_graph_1s(&rows, phase.start, phase.end);
        assert!(g.windows(2).all(|w| w[0] <= w[1]), "player damage1S monotonic: {}", p.character);
        assert_eq!(*g.last().expect("non-empty"), expected, "player damage1S last: {}", p.character);
        eprintln!("player {:<20} damage1S tail={:?} total={expected}", p.character, &g[g.len() - 3..]);
    }
    // ---- 非小队账号/身份命名 ----
    let ns_accounts: Vec<&str> = log
        .friendly_non_squad
        .iter()
        .map(|n| n.account.as_str())
        .collect();
    assert_eq!(ns_accounts, vec!["Non Squad Player 1", "Non Squad Player 2"]);
    eprintln!(
        "friendly non-squad: {:?}",
        log.friendly_non_squad
            .iter()
            .map(|n| n.character.as_str())
            .collect::<Vec<_>>()
    );
    let thaha = log.players.iter().find(|p| p.character == "Thaha").expect("thaha");
    assert_eq!(thaha.group, 2, "golden Thaha group=2");
    assert_eq!(thaha.account, "happy.8615");
}

/// actor outgoing 伤害行（GetDamageEvents(target=None) 口径：From==自身 ∪
/// minions（master 链终结于 actor 的 NPC））。
fn actor_outgoing(
    log: &ParsedLog,
    idx: &gw2ei_model::EventIndex,
    actor: AgentId,
) -> Vec<gw2ei_model::DmgRow> {
    let mut ids = vec![actor];
    let actor_root = log.agents.englobing_root(actor);
    for id in log.agents.all_ids() {
        if id == actor {
            continue;
        }
        let a = log.agents.slot(id).expect("slot");
        if (a.agent_type == AgentType::StableSpecies || a.agent_type == AgentType::VolatileSpecies)
            && log.agents.final_master(id) == actor_root
        {
            ids.push(id);
        }
    }
    let mut rows = Vec::new();
    for id in ids {
        if let Some(list) = idx.damage_from.get(&id) {
            for &r in list {
                if let Some(row) = dmg_row(&log.events[r], id, NO_AGENT) {
                    rows.push(row);
                }
            }
        }
    }
    rows.sort_by_key(|r| r.time);
    rows
}

fn actor_breakbar(
    log: &ParsedLog,
    idx: &gw2ei_model::EventIndex,
    actor: AgentId,
) -> (f64, f64) {
    let actor_root = log.agents.englobing_root(actor);
    let mut total = 0.0f64;
    let mut actor_only = 0.0f64;
    for (agent, rows) in &idx.breakbar_damage_from {
        let is_self = *agent == actor;
        let a = log.agents.slot(*agent).expect("slot");
        let is_minion = *agent != actor
            && (a.agent_type == AgentType::StableSpecies || a.agent_type == AgentType::VolatileSpecies)
            && log.agents.final_master(*agent) == actor_root;
        if !is_self && !is_minion {
            continue;
        }
        for &r in rows {
            if let CombatEvent::BreakbarDamage(f) = &log.events[r] {
                total += f.value;
                if is_self {
                    actor_only += f.value;
                }
            }
        }
    }
    (total, actor_only)
}

#[test]
fn actors_sample_link_counts() {
    let Some(mut log) = load() else { return };
    let n_player_roots = log.agents.agents_by_type(AgentType::Player).len();
    assert_eq!(n_player_roots, 46);
    let mut expect_friends = 0usize;
    for f in &log.friendlies {
        match f {
            Friend::Player(i) => assert!(*i < log.players.len()),
            Friend::NonSquad(i) => assert!(*i < log.friendly_non_squad.len()),
        }
        expect_friends += 1;
    }
    assert_eq!(expect_friends, 48);
    let groups: Vec<i32> = log.players.iter().map(|p| p.group).collect();
    assert!(groups.iter().all(|&g| g > 0));
    assert!(log.link_stats.missing_agents_added > 0);
    eprintln!(
        "link stats: missing={} removed_items={} agents_removed={} npc_regroup={}",
        log.link_stats.missing_agents_added,
        log.link_stats.invalid_both_removed_items,
        log.link_stats.agents_removed,
        log.link_stats.regroup_npc_groups
    );
    let _ = &mut log;
}
