//! Agent 链接/归并/分裂流水线（P2a）。
//!
//! 对齐 `EvtcParser.CompleteAgentsAndLogData`（EvtcParser.cs:1144-1360）+
//! `AgentManipulationHelper.cs`（快照 `3b7278f9b`）：
//!
//! 1. 补缺 agent（事件引用但 agent 区没有的地址 → "UNKNOWN {addr}"）
//! 2. 逐事件链接：AgentChange 重定向改写 → UpdateAgentData（InstID 首见 /
//!    aware 区间扩展 / 同地址多 agent 冲突试下一个）
//! 3. 孤儿修正（instid ±300ms aware 候选改写地址）
//! 4. 双向无效（src∧dst 都无效）事件移除
//! 5. 删无效 agent（LastAware==MaxValue 或 LastAware<FirstAware）
//! 6. 加 fake Environment agent
//! 7. extension AdjustCombatEvent —— P2 无扩展内容，显式跳过（记录在案）
//! 8. master 链（src/dst_master_instid → FindAgentMaster）
//! 9. RegroupSameAgentsAndDetermineTeams（NPC 同 InstID 归并 / 非小队玩家
//!    归并 + 友方判定 / 玩家按账号归并）
//! 10. 玩家按 (spec, subgroup) EnterCombat 切分（SplitPlayerPerSpecSubgroupAndSwap）
//!
//! 事件重写（OverrideSrc/DstAgent）在 `EvtcCombatItem` 上原地改地址；
//! 归并后地址保持组内首元素的地址（C# `AgentItem(AgentItem other)` 拷贝构造
//! 保留 `Agent` 字段），因此解析语义在重写后自洽。

use std::collections::{BTreeMap, BTreeSet};

use gw2ei_parse::{EvtcCombatItem, EvtcRawLog, StateChange};

use crate::agent::{
    AgentId, AgentItem, AgentMergedFrom, AgentTable, AgentType, NO_AGENT,
    agent_from_raw_with, species,
};
use crate::error::ModelError;
use crate::spec::{NoSpecCatalog, Spec, SpecCatalog};

/// 链接阶段统计（可观测性增强，C# 无对应字段）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub missing_agents_added: u64,
    pub orphan_src_fixed: u64,
    pub orphan_dst_fixed: u64,
    pub invalid_both_removed_items: u64,
    pub agents_removed: u64,
    pub master_link_attempts: u64,
    pub regroup_npc_groups: u64,
    pub regroup_non_squad_groups: u64,
    pub regroup_player_groups: u64,
    pub split_pieces_created: u64,
}

/// 位置行坐标解包（C# `MovementEvent.UnpackMovementData`：DstAgent 低 8B =
/// 两个 f32（低 4B=x、高 4B=y），Value 位型 f32=z）。位型重解释恒成功，
/// 可为 NaN/Inf（C# 在 Regroup 距离比较处同样保留，NaN 比较恒 false）。
pub(crate) fn unpack_position(item: &EvtcCombatItem) -> (f64, f64, f64) {
    let bytes = item.dst_agent.to_le_bytes();
    let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let z = f32::from_bits(item.value as u32);
    (f64::from(x), f64::from(y), f64::from(z))
}

/// 完整链接流水线（在 `log` 上原地改写地址/移除事件）。返回最终 AgentTable。
pub fn complete_agents(log: &mut EvtcRawLog) -> Result<(AgentTable, LinkStats), ModelError> {
    complete_agents_with(log, &NoSpecCatalog)
}

/// `complete_agents` 的带 SpecList 版本：`catalog` 提供 (prof, elite) → Spec
/// （C# `GW2APIController.GetSpec`；P2b 由 content/SpecList.json 实现）。
pub fn complete_agents_with(
    log: &mut EvtcRawLog,
    catalog: &dyn SpecCatalog,
) -> Result<(AgentTable, LinkStats), ModelError> {
    let mut stats = LinkStats::default();
    let mut table = AgentTable::new();
    for raw_agent in &log.agents {
        agent_from_raw_with(raw_agent, &mut table, catalog);
    }
    let mut addresses: BTreeSet<u64> = table
        .all_ids()
        .into_iter()
        .map(|id| table.slot(id).expect("slot").address)
        .collect();

    // ===== 1. 补缺 agent（EvtcParser.cs:1147-1160）=====
    {
        let mut missing: BTreeSet<u64> = BTreeSet::new();
        for item in &log.combat_events {
            if item.src_is_agent() {
                missing.insert(item.src_agent);
            }
            if item.dst_is_agent() {
                missing.insert(item.dst_agent);
            }
        }
        missing.remove(&0);
        missing.retain(|addr| !addresses.contains(addr));
        stats.missing_agents_added = missing.len() as u64;
        for addr in missing {
            let name = format!("UNKNOWN {addr}");
            let mut ag = AgentItem {
                address: addr,
                name: name.clone(),
                agent_type: AgentType::StableSpecies,
                spec: Spec::Npc,
                base_spec: Spec::Npc,
                id: species::NON_IDENTIFIED,
                instid: 0,
                toughness: 0,
                healing: 0,
                condition: 0,
                concentration: 0,
                hitbox_width: 0,
                hitbox_height: 0,
                first_aware: i64::MAX,
                last_aware: i64::MAX,
                master: NO_AGENT,
                englobing: NO_AGENT,
                englobed: Vec::new(),
                regrouped: Vec::new(),
                is_fake: false,
                is_not_in_squad_friendly_player: false,
                is_unknown: false,
                character: name,
                account: None,
                group: None,
                unamed: false,
            };
            ag.character.clone_from(&ag.name);
            table.push_agent(ag);
            addresses.insert(addr);
        }
    }

    // ===== 2. 逐事件链接（EvtcParser.cs:1161-1231）=====
    let mut invalid_src: BTreeSet<u64> = BTreeSet::new();
    let mut invalid_dst: BTreeSet<u64> = BTreeSet::new();
    let mut orphan_src: Vec<usize> = Vec::new();
    let mut orphan_dst: Vec<usize> = Vec::new();
    {
        // 同地址列表按 (FirstAware, 槽位序) 稳定升序（agentsLookup :1161-1166）。
        let mut by_addr: BTreeMap<u64, Vec<AgentId>> = BTreeMap::new();
        for id in table.ids_sorted_by_first_aware() {
            let a = table.slot(id).expect("slot");
            by_addr.entry(a.address).or_default().push(id);
        }
        for (idx, item) in log.combat_events.iter_mut().enumerate() {
            if item.src_is_agent() {
                if let Some(&new_addr) = log.agent_redirection.get(&item.src_agent) {
                    item.src_agent = new_addr;
                }
                let mut updated = false;
                if let Some(agents) = by_addr.get(&item.src_agent) {
                    for &aid in agents {
                        updated = update_agent_data(
                            &mut table,
                            aid,
                            item.time,
                            item.src_instid,
                            agents.len() > 1,
                        );
                        if updated {
                            break;
                        }
                    }
                }
                if !updated && item.src_instid != 0 {
                    invalid_src.insert(idx as u64);
                } else if !by_addr.contains_key(&item.src_agent) && item.src_instid > 0 {
                    orphan_src.push(idx);
                }
            }
            if item.dst_is_agent() {
                if let Some(&new_addr) = log.agent_redirection.get(&item.dst_agent) {
                    item.dst_agent = new_addr;
                }
                let mut updated = false;
                if let Some(agents) = by_addr.get(&item.dst_agent) {
                    for &aid in agents {
                        updated = update_agent_data(
                            &mut table,
                            aid,
                            item.time,
                            item.dst_instid,
                            agents.len() > 1,
                        );
                        if updated {
                            break;
                        }
                    }
                }
                if !updated && item.dst_instid != 0 {
                    invalid_dst.insert(idx as u64);
                } else if !by_addr.contains_key(&item.dst_agent) && item.dst_instid > 0 {
                    orphan_dst.push(idx);
                }
            }
        }
    }

    // ===== 3. 孤儿修正（EvtcParser.cs:1232-1264）=====
    if !orphan_src.is_empty() || !orphan_dst.is_empty() {
        let mut by_instid: BTreeMap<u16, Vec<AgentId>> = BTreeMap::new();
        for id in table.ids_sorted_by_first_aware() {
            let a = table.slot(id).expect("slot");
            by_instid.entry(a.instid).or_default().push(id);
        }
        let find_candidate = |by_instid: &BTreeMap<u16, Vec<AgentId>>,
                              table: &AgentTable,
                              instid: u16,
                              time: i64|
         -> Option<AgentId> {
            by_instid.get(&instid).and_then(|list| {
                list.iter().copied().find(|&aid| {
                    let a = table.slot(aid).expect("slot");
                    a.in_aware_times(time - 300) || a.in_aware_times(time + 300)
                })
            })
        };
        for idx in orphan_src {
            let (instid, time) = {
                let item = &log.combat_events[idx];
                (item.src_instid, item.time)
            };
            if let Some(aid) = find_candidate(&by_instid, &table, instid, time) {
                let addr = table.slot(aid).expect("slot").address;
                log.combat_events[idx].src_agent = addr;
                update_agent_data(&mut table, aid, time, 0, false);
                stats.orphan_src_fixed += 1;
            }
        }
        for idx in orphan_dst {
            let (instid, time) = {
                let item = &log.combat_events[idx];
                (item.dst_instid, item.time)
            };
            if let Some(aid) = find_candidate(&by_instid, &table, instid, time) {
                let addr = table.slot(aid).expect("slot").address;
                log.combat_events[idx].dst_agent = addr;
                update_agent_data(&mut table, aid, time, 0, false);
                stats.orphan_dst_fixed += 1;
            }
        }
    }

    // ===== 4. 双向无效移除（EvtcParser.cs:1265-1274）=====
    let both_invalid: BTreeSet<u64> = invalid_src.intersection(&invalid_dst).copied().collect();
    stats.invalid_both_removed_items = both_invalid.len() as u64;
    if !both_invalid.is_empty() {
        let mut kept = Vec::with_capacity(log.combat_events.len());
        for (idx, item) in log.combat_events.drain(..).enumerate() {
            if !both_invalid.contains(&(idx as u64)) {
                kept.push(item);
            }
        }
        log.combat_events = kept;
    }

    // ===== 5. 删无效 agent（EvtcParser.cs:1275）=====
    {
        let remove: Vec<AgentId> = table
            .all_ids()
            .into_iter()
            .filter(|&id| {
                let a = table.slot(id).expect("slot");
                !(a.last_aware != i64::MAX && a.last_aware - a.first_aware >= 0)
            })
            .collect();
        stats.agents_removed = remove.len() as u64;
        for id in remove {
            table.remove_agent(id);
        }
    }

    // ===== 6. fake Environment agent（EvtcParser.cs:1278-1279）=====
    add_environment_agent(&mut table, &mut addresses, log.log_end_time);

    // ===== 8. master 链（EvtcParser.cs:1298-1309）=====
    {
        let items = log.combat_events.clone();
        for item in &items {
            if item.src_is_agent() && item.src_master_instid != 0 {
                find_agent_master(&mut table, item.time, item.src_master_instid, item.src_agent);
                stats.master_link_attempts += 1;
            }
            if item.dst_is_agent() && item.dst_master_instid != 0 {
                find_agent_master(&mut table, item.time, item.dst_master_instid, item.dst_agent);
                stats.master_link_attempts += 1;
            }
        }
    }

    // ===== 9. RegroupSameAgentsAndDetermineTeams =====
    regroup_same_agents_and_determine_teams(&mut table, &mut log.combat_events, &mut stats);

    // ===== 玩家存在性检查（EvtcParser.cs:1314-1317）=====
    if table.agents_by_type(AgentType::Player).is_empty() {
        return Err(ModelError::NoPlayersFound);
    }

    // ===== 10. SplitPlayerPerSpecSubgroupAndSwap（EvtcParser.cs:1329-1354）=====
    split_players_by_enter_combat(&mut table, &log.combat_events, catalog, &mut stats);

    table.finalize();
    Ok((table, stats))
}

/// fake Environment agent（AgentData.AddCustomNPCAgent；C# 随机地址/instid
/// 用唯一性循环 —— Rust 用确定性 LCG，仅要求唯一）。
fn add_environment_agent(table: &mut AgentTable, addresses: &mut BTreeSet<u64>, log_end: i64) {
    let mut rng = 0x1234_5678_9abc_def0u64;
    let used_addrs = addresses.clone();
    let addr = loop {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let candidate = (u64::from(u32::MAX) / 2) + (rng >> 33) % (u64::from(u32::MAX) / 2);
        if candidate != 0 && !used_addrs.contains(&candidate) {
            break candidate;
        }
    };
    addresses.insert(addr);
    let used_instids: BTreeSet<u16> = table
        .all_ids()
        .into_iter()
        .map(|id| table.slot(id).expect("slot").instid)
        .collect();
    let mut instid = 0u16;
    while instid == 0 || used_instids.contains(&instid) {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        instid = (u16::MAX / 2) + (rng >> 48) as u16 % (u16::MAX / 2);
    }
    let name = "Environment".to_string();
    let mut ag = AgentItem {
        address: addr,
        name: name.clone(),
        agent_type: AgentType::StableSpecies, // AddCustomNPCAgent 恒 StableSpecies
        spec: Spec::Gadget,
        base_spec: Spec::Gadget,
        id: species::ENVIRONMENT,
        instid,
        toughness: 0,
        healing: 0,
        condition: 0,
        concentration: 0,
        hitbox_width: 0,
        hitbox_height: 0,
        first_aware: 0,
        last_aware: log_end,
        master: NO_AGENT,
        englobing: NO_AGENT,
        englobed: Vec::new(),
        regrouped: Vec::new(),
        is_fake: true,
        is_not_in_squad_friendly_player: false,
        is_unknown: false,
        character: name,
        account: None,
        group: None,
        unamed: false,
    };
    ag.character.clone_from(&ag.name);
    table.push_agent(ag);
}

/// `UpdateAgentData`（EvtcParser.cs:986-1009）。
fn update_agent_data(
    table: &mut AgentTable,
    agent: AgentId,
    log_time: i64,
    instid: u16,
    check_instid: bool,
) -> bool {
    {
        let a = table.slot(agent).expect("slot");
        if instid != 0 && a.instid != 0 && check_instid && a.instid != instid {
            return false;
        }
    }
    let a = table.slot_mut(agent).expect("slot");
    if instid != 0 && a.instid == 0 {
        a.instid = instid;
    }
    if a.last_aware == i64::MAX {
        a.first_aware = log_time;
        a.last_aware = log_time;
    } else {
        a.first_aware = a.first_aware.min(log_time);
        a.last_aware = a.last_aware.max(log_time);
    }
    true
}

/// `FindAgentMaster`（EvtcParser.cs:1018-1029）。
fn find_agent_master(table: &mut AgentTable, log_time: i64, master_instid: u16, minion_addr: u64) {
    let master = table.agent_by_instid(master_instid, log_time);
    if master != NO_AGENT {
        let minion = table.resolve_agent(minion_addr, log_time);
        if minion != NO_AGENT {
            table.set_master(minion, master);
        }
    }
}

/// 按地址/时间解析后的事件↔agent 分组（src/dst 两侧），供归并重写。
fn build_item_side_maps(
    table: &mut AgentTable,
    items: &[EvtcCombatItem],
) -> (BTreeMap<AgentId, Vec<usize>>, BTreeMap<AgentId, Vec<usize>>) {
    let mut src: BTreeMap<AgentId, Vec<usize>> = BTreeMap::new();
    let mut dst: BTreeMap<AgentId, Vec<usize>> = BTreeMap::new();
    for (idx, item) in items.iter().enumerate() {
        if item.src_is_agent() {
            let a = table.resolve_agent(item.src_agent, item.time);
            if a != NO_AGENT {
                src.entry(a).or_default().push(idx);
            }
        }
        if item.dst_is_agent() {
            let a = table.resolve_agent(item.dst_agent, item.time);
            if a != NO_AGENT {
                dst.entry(a).or_default().push(idx);
            }
        }
    }
    (src, dst)
}

/// 归并主体（AgentManipulationHelper.cs:306-492）。
fn regroup_same_agents_and_determine_teams(
    table: &mut AgentTable,
    items: &mut [EvtcCombatItem],
    stats: &mut LinkStats,
) {
    // squadCombatStart/End 时间桶边界（:310-314）
    let mut squad_boundaries: Vec<i64> = vec![i64::MIN];
    squad_boundaries.extend(
        items
            .iter()
            .filter(|x| {
                matches!(
                    x.state_change,
                    StateChange::SquadCombatStart | StateChange::SquadCombatEnd
                )
            })
            .map(|x| x.time),
    );
    squad_boundaries.push(i64::MAX);
    let state_time_of = |first_aware: i64, last_aware: i64| -> i64 {
        let half = (first_aware + last_aware) / 2;
        squad_boundaries
            .iter()
            .rev()
            .find(|&&b| b <= half)
            .copied()
            .unwrap_or(i64::MIN)
    };

    // PoV + 位置事件（:315-333）
    let pov_item = items
        .iter()
        .find(|x| x.state_change == StateChange::PointOfView)
        .copied();
    let mut pov_agent = NO_AGENT;
    let mut pov_positions: Vec<(i64, f64, f64)> = Vec::new();
    if let Some(pov) = pov_item {
        pov_agent = table.resolve_agent(pov.src_agent, pov.time);
        if pov_agent != NO_AGENT {
            for item in items.iter().filter(|x| x.is_position()) {
                if table.resolve_agent(item.src_agent, item.time) == pov_agent {
                    pov_positions.push((item.time, unpack_position(item).0, unpack_position(item).1));
                }
            }
            pov_positions.sort_by_key(|p| p.0);
        }
    }
    // 每个 NPC/agent 的 last/first 位置（含 NaN —— C# LastOrDefault 不滤值）
    let mut first_pos: BTreeMap<AgentId, (i64, f64, f64)> = BTreeMap::new();
    let mut last_pos: BTreeMap<AgentId, (i64, f64, f64)> = BTreeMap::new();
    for item in items.iter().filter(|x| x.is_position()) {
        let a = table.resolve_agent(item.src_agent, item.time);
        if a == NO_AGENT {
            continue;
        }
        let p = (item.time, unpack_position(item).0, unpack_position(item).1);
        first_pos.entry(a).or_insert(p);
        last_pos.insert(a, p);
    }
    let dist_xy = |(x1, y1): (f64, f64), (x2, y2): (f64, f64)| -> f64 {
        ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt()
    };

    let (src_items, dst_items) = build_item_side_maps(table, items);
    let distance_threshold = 4980.0f64;

    // ===== NPC 归并（:337-428）=====
    {
        let mut npcs_by_instid: BTreeMap<u16, Vec<AgentId>> = BTreeMap::new();
        for id in table.agents_by_type(AgentType::StableSpecies) {
            let a = table.slot(id).expect("slot");
            npcs_by_instid.entry(a.instid).or_default().push(id);
        }
        for mut list in npcs_by_instid.into_values() {
            if list.len() < 2 {
                continue;
            }
            list.sort_by_key(|&id| {
                let a = table.slot(id).expect("slot");
                (a.first_aware, id)
            });
            let no_pov = pov_agent == NO_AGENT || pov_positions.is_empty();
            let mut group: Vec<AgentId> = Vec::with_capacity(4);
            let mut previous = list[0];
            let mut previous_state = state_time_of(
                table.slot(previous).expect("slot").first_aware,
                table.slot(previous).expect("slot").last_aware,
            );
            for &cur in &list {
                let cur_state = state_time_of(
                    table.slot(cur).expect("slot").first_aware,
                    table.slot(cur).expect("slot").last_aware,
                );
                let cur_agent = table.slot(cur).expect("cur");
                let prev_agent = table.slot(previous).expect("prev");
                let equal_prev = cur_agent.could_be_equal(prev_agent) && cur_state == previous_state;
                let mut merge = false;
                if equal_prev {
                    if no_pov {
                        // 无 PoV 分支（:339-369）
                        let fm = table.final_master(cur);
                        let fm_is_player = fm != NO_AGENT
                            && table.slot(fm).map(|a| a.agent_type.is_player()).unwrap_or(false);
                        merge = !fm_is_player;
                    } else {
                        // PoV 距离分支（:370-427）：goNext=false（距离超阈值）
                        // 才把 cur 并入组（两个距离检查任一命中即并）。
                        let mut go_next = true;
                        if let Some(&agent_pos) = last_pos.get(&previous)
                            && let Some(&next_pov) = pov_positions
                                .iter()
                                .find(|p| p.0 > table.slot(previous).expect("slot").last_aware)
                                && dist_xy((next_pov.1, next_pov.2), (agent_pos.1, agent_pos.2))
                                    > distance_threshold
                                {
                                    go_next = false;
                                }
                        if go_next
                            && let Some(&agent_pos) = first_pos.get(&cur)
                                && let Some(&prev_pov) = pov_positions
                                    .iter()
                                    .rev()
                                    .find(|p| p.0 < table.slot(cur).expect("cur").first_aware)
                                    && dist_xy((prev_pov.1, prev_pov.2), (agent_pos.1, agent_pos.2))
                                        > distance_threshold
                                    {
                                        go_next = false;
                                    }
                        merge = !go_next;
                    }
                }
                if merge {
                    group.push(cur);
                } else {
                    if group.len() > 1 {
                        regroup_agents(table, &group, items, &src_items, &dst_items);
                        stats.regroup_npc_groups += 1;
                    }
                    group = vec![cur];
                    previous_state = cur_state;
                }
                previous = cur;
            }
            if group.len() > 1 {
                regroup_agents(table, &group, items, &src_items, &dst_items);
                stats.regroup_npc_groups += 1;
            }
        }
    }

    // ===== 非小队玩家（:429-477）=====
    let non_squad = table.agents_by_type(AgentType::NonSquadPlayer);
    if !non_squad.is_empty() {
        let team_events_by_addr: BTreeMap<u64, Vec<(i64, i64)>> = {
            let mut m: BTreeMap<u64, Vec<(i64, i64)>> = BTreeMap::new();
            for item in items.iter().filter(|x| x.state_change == StateChange::TeamChange) {
                m.entry(item.src_agent)
                    .or_default()
                    .push((item.dst_agent as i64, item.value as i64));
            }
            m
        };
        let mut squad_teams: Vec<i64> = Vec::new();
        for pid in table.agents_by_type(AgentType::Player) {
            let a = table.slot(pid).expect("slot");
            if let Some(events) = team_events_by_addr.get(&a.address) {
                // TeamChangeOnDespawn(20240612) 之后 ComingFrom 恒有值
                for &(into, from) in events {
                    if into != 0 {
                        squad_teams.push(into);
                    }
                    if from != 0 {
                        squad_teams.push(from);
                    }
                }
            }
        }
        if !squad_teams.is_empty() {
            let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
            for t in &squad_teams {
                *counts.entry(*t).or_insert(0) += 1;
            }
            // C# GroupBy+OrderByDescending(Count).First()
            let most_common = *counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(t, _)| t)
                .expect("non-empty squad teams");
            for nid in &non_squad {
                let a = table.slot(*nid).expect("slot");
                if let Some(events) = team_events_by_addr.get(&a.address) {
                    let has_squad_team = events.iter().any(|&(into, from)| {
                        into != 0 && into == most_common
                            || from != 0 && from == most_common
                    });
                    if has_squad_team {
                        table.slot_mut(*nid).expect("slot").is_not_in_squad_friendly_player = true;
                    }
                }
            }
        }
        let mut by_instid: BTreeMap<u16, Vec<AgentId>> = BTreeMap::new();
        for id in non_squad {
            let a = table.slot(id).expect("slot");
            by_instid.entry(a.instid).or_default().push(id);
        }
        for group in by_instid.into_values() {
            if group.len() > 1 {
                regroup_agents(table, &group, items, &src_items, &dst_items);
                stats.regroup_non_squad_groups += 1;
            }
        }
    }

    // ===== 玩家按账号归并（:478-490）=====
    {
        let mut by_account: BTreeMap<String, Vec<AgentId>> = BTreeMap::new();
        for id in table.agents_by_type(AgentType::Player) {
            let a = table.slot(id).expect("slot");
            by_account
                .entry(a.account.clone().unwrap_or_default())
                .or_default()
                .push(id);
        }
        for group in by_account.into_values() {
            if group.len() > 1 {
                regroup_agents(table, &group, items, &src_items, &dst_items);
                stats.regroup_player_groups += 1;
            }
        }
    }

}

/// `RegroupAgents`（AgentManipulationHelper.cs:283-304）：把组内 agent 合并为
/// 一个新 agent（拷贝首元素，aware 取并集），事件地址重写为合并者地址。
fn regroup_agents(
    table: &mut AgentTable,
    group: &[AgentId],
    items: &mut [EvtcCombatItem],
    src_items: &BTreeMap<AgentId, Vec<usize>>,
    dst_items: &BTreeMap<AgentId, Vec<usize>>,
) {
    let first = group[0];
    let mut new_agent = table.slot(first).expect("slot").clone();
    new_agent.first_aware = group
        .iter()
        .map(|&id| table.slot(id).expect("slot").first_aware)
        .min()
        .unwrap_or(0);
    new_agent.last_aware = group
        .iter()
        .map(|&id| table.slot(id).expect("slot").last_aware)
        .max()
        .unwrap_or(0);
    new_agent.englobed = Vec::new();
    new_agent.englobing = NO_AGENT;
    new_agent.regrouped = Vec::new();
    for &id in group {
        // 事件重写（copy ctor 保留首元素地址 -> 合并者地址 = 首元素地址）
        if let Some(list) = src_items.get(&id) {
            for &idx in list {
                items[idx].src_agent = new_agent.address;
            }
        }
        if let Some(list) = dst_items.get(&id) {
            for &idx in list {
                items[idx].dst_agent = new_agent.address;
            }
        }
        let a = table.slot(id).expect("slot");
        // C# `AddRegroupedFrom`（AgentItem.cs:604-608）：记录合并来源区间，
        // 供匿名化与 Split 的 couple.from 选择。
        new_agent.regrouped.push(AgentMergedFrom {
            first_aware: a.first_aware,
            last_aware: a.last_aware,
            name: a.name.clone(),
            character: a.character.clone(),
            spec: a.spec,
            base_spec: a.base_spec,
            agent_type: a.agent_type,
            toughness: a.toughness,
            healing: a.healing,
            condition: a.condition,
            concentration: a.concentration,
            is_fake: a.is_fake,
        });
    }
    // SwapMasters（AgentManipulationHelper.cs:299 + AgentData.cs:316-337）：
    // 组内 agent 自身是 minion（master != null）时 master 改指合并者。
    // C# 在 newAgent 未入表前把引用指过去；Rust 先入表再改指，效果一致。
    let new_id = table.push_agent(new_agent);
    for &id in group {
        let has_master = table.slot(id).map(|a| a.master != NO_AGENT).unwrap_or(false);
        if has_master {
            table.set_master(id, new_id);
        }
    }
    for &id in group {
        table.remove_agent(id);
    }
}

/// `SplitPlayerPerSpecSubgroupAndSwap`（AgentManipulationHelper.cs:200-281）。
/// 玩家（AgentType::Player 根）按 EnterCombat (spec, subgroup) 序列切分；
/// 每段生成 englobed agent（新随机地址，`AddCustomAgentFrom` 语义）。
///
/// 已知局限（P2b 消除）：spec 解码是本地子集 —— 现代精英专精为
/// `Spec::Unknown`，C# 会跳过这类 enter 事件（`enterCombat.Spec == Unknown`
/// continue），语义一致；SpecList.json 载入后自动精确化。
/// `SplitPlayerPerSpecSubgroupAndSwap`（AgentManipulationHelper.cs:200-281）。
/// 玩家（AgentType::Player 根）按 EnterCombat (spec, subgroup) 序列切分；
/// 每段生成 englobed agent（新随机地址，`AddCustomAgentFrom` 语义）。
///
/// 已知局限（P2b 消除）：spec 解码是本地子集 —— 现代精英专精为
/// `Spec::Unknown`，C# 的过滤条件 `enterCombat.Spec == Unknown → continue`
/// 与本侧一致（都不切分），SpecList.json 载入后自动精确化。
fn split_players_by_enter_combat(
    table: &mut AgentTable,
    items: &[EvtcCombatItem],
    catalog: &dyn SpecCatalog,
    stats: &mut LinkStats,
) {
    // 每玩家的 enter/exit（解析到根；保留流序 = 稳定时间序）
    let players = table.agents_by_type(AgentType::Player);
    for root in players {
        let mut enters: Vec<(i64, u64, Spec)> = Vec::new(); // (time, subgroup, spec)
        let mut exits: Vec<i64> = Vec::new();
        for item in items {
            match item.state_change {
                StateChange::EnterCombat => {
                    if table.resolve_agent(item.src_agent, item.time) == root {
                        let spec = catalog.spec_of(item.value as u32, item.buff_dmg as u32);
                        enters.push((item.time, item.dst_agent, spec));
                    }
                }
                StateChange::ExitCombat
                    if table.resolve_agent(item.src_agent, item.time) == root => {
                        exits.push(item.time);
                    }
                _ => {}
            }
        }
        enters.sort_by_key(|e| e.0);
        exits.sort_by_key(|e| *e);
        let pieces = split_one_player(table, root, &enters, &exits);
        stats.split_pieces_created += pieces as u64;
    }
}

/// 单玩家切分核心（AgentManipulationHelper.cs:200-281）。
/// 返回生成的 englobed 段数。
fn split_one_player(
    table: &mut AgentTable,
    root: AgentId,
    enters: &[(i64, u64, Spec)],
    exits: &[i64],
) -> usize {
    let root_item = table.slot(root).expect("root");
    let mut previous_spec = root_item.spec;
    let mut previous_group = root_item.group.unwrap_or(0) as i64;
    // 切分点列表（:205-240）：spec 或 subgroup 与上次不同才产生条目；
    // ignore0Subgroups（Type==Player）-> subgroup==0 跳过；spec Unknown 跳过。
    let mut list: Vec<(i64, i64, Spec)> = Vec::new(); // (start, group, spec)
    for &(time, subgroup, spec) in enters {
        if subgroup == 0 || spec == Spec::Unknown {
            continue;
        }
        let subgroup = subgroup as i64;
        if spec != previous_spec || subgroup != previous_group {
            previous_spec = spec;
            previous_group = subgroup;
            list.push((time, subgroup, spec));
        }
    }
    if list.is_empty() {
        return 0;
    }
    list.sort_by_key(|e| e.0);
    let mut pieces = 0usize;
    let mut previous_player = root;
    let mut first_split = true;
    let mut previous_start: i64 = 0;
    let mut prev_spec = table.slot(root).expect("root").spec;
    let mut prev_group = table.slot(root).expect("root").group.unwrap_or(0) as i64;
    for &(start0, group, spec) in &list {
        if spec == prev_spec && group == prev_group {
            continue;
        }
        prev_spec = spec;
        prev_group = group;
        let mut start = start0;
        // `exitCombatEvents.LastOrDefault(t < start && t > previousStart)`（:254）
        let prev_exit = exits
            .iter()
            .rev()
            .find(|&&t| t < start && t > previous_start)
            .copied();
        if let Some(exit_time) = prev_exit {
            // 变化点在 previous exit 与当前 start 之间 -> 取中点（:258）
            start = (exit_time + 1 + start) / 2;
        }
        let end = table.slot(previous_player).expect("prev").last_aware;
        if first_split {
            first_split = false;
            if prev_exit.is_none() {
                // 首段不知何时变化 -> 取 aware 时间中点（:267）
                start = (table.slot(previous_player).expect("prev").first_aware + start) / 2;
            }
            // piece A = [root.FirstAware, start-1]，spec = 根 spec（:269-270）
            let piece_a = add_custom_agent_from(
                table,
                previous_player,
                table.slot(previous_player).expect("prev").first_aware,
                start - 1,
                table.slot(previous_player).expect("prev").spec,
            );
            table.set_englobing(piece_a, root);
            previous_player = piece_a;
        }
        // piece B = [start, end]（:273-274）；couple.from 恒为根
        //（Regrouped 段选择留 SpecList 时代的 P2b 精确化，快照字段已在案）
        let piece_b = add_custom_agent_from(table, root, start, end, spec);
        table.set_englobing(piece_b, root);
        // 前段收窄到 start-1（:276）
        if let Some(p) = table.slot_mut(previous_player) {
            p.last_aware = start - 1;
        }
        previous_player = piece_b;
        previous_start = start;
        pieces += 1;
    }
    pieces
}

/// `AgentData.AddCustomAgentFrom`（AgentData.cs:72-89）：拷贝 agent 的
/// 名字/类型/数值字段，spec 指定，ID=0，新随机地址与 instid，aware 定区间。
fn add_custom_agent_from(
    table: &mut AgentTable,
    from: AgentId,
    start: i64,
    end: i64,
    spec: Spec,
) -> AgentId {
    let src = table.slot(from).expect("from").clone();
    let mut ag = AgentItem {
        address: 0,
        name: src.name.clone(),
        agent_type: src.agent_type,
        spec,
        base_spec: spec.base_spec(),
        id: 0,
        instid: 0,
        toughness: src.toughness,
        healing: src.healing,
        condition: src.condition,
        concentration: src.concentration,
        hitbox_width: src.hitbox_width,
        hitbox_height: src.hitbox_height,
        first_aware: start,
        last_aware: end,
        master: NO_AGENT,
        englobing: NO_AGENT,
        englobed: Vec::new(),
        regrouped: Vec::new(),
        is_fake: src.is_fake,
        is_not_in_squad_friendly_player: src.is_not_in_squad_friendly_player,
        is_unknown: false,
        character: src.character.clone(),
        account: src.account.clone(),
        group: src.group,
        unamed: src.unamed,
    };
    ag.character = ag.name.split('\0').next().unwrap_or("").to_string();
    let new_id = table.push_agent(ag);
    // 新随机地址/instid（不与现存冲突；C# Random 语义，值不进 JSON 对拍面）
    let mut rng = 0x9e37_79b9_7f4a_7c15u64.wrapping_mul(new_id as u64 + 1);
    loop {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let addr = (u64::from(u32::MAX) / 2) + (rng >> 33) % (u64::from(u32::MAX) / 2);
        if addr == 0 {
            continue;
        }
        let clash = table
            .all_ids()
            .into_iter()
            .any(|id| table.slot(id).expect("slot").address == addr);
        if !clash {
            table.slot_mut(new_id).expect("new").address = addr;
            break;
        }
    }
    loop {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let instid = (u16::MAX / 2) + (rng >> 48) as u16 % (u16::MAX / 2);
        if instid == 0 {
            continue;
        }
        let clash = table
            .all_ids()
            .into_iter()
            .any(|id| table.slot(id).expect("slot").instid == instid);
        if !clash {
            table.slot_mut(new_id).expect("new").instid = instid;
            break;
        }
    }
    new_id
}

