//! WvW 地图静态数据（C# `WvWHelper.cs` + WvWLogic 常量）：objective 类型表
//! 与 wvWMapData/teamMap 构建（P4）。只保留 JSON 需要的面（type 判过滤）；
//! continentPos 仅 HTML 装饰用，不迁移。

use gw2ei_model::events::{CombatEvent, WvWObjectiveStatusEventFields};

use crate::dto_a::{WvwMapData, WvwObjectiveData};

/// TeamGUID 反查表（C# `TeamGUIDEventsByTeamID`，TeamGUIDEvent.cs：
/// TeamID=(ulong)ContentID；字典后写覆盖 → 流内同 team id 最后一行）。
pub fn team_guid_by_id(
    log: &gw2ei_model::ParsedLog,
) -> std::collections::BTreeMap<i64, String> {
    let mut map = std::collections::BTreeMap::new();
    for evt in &log.metadata.id_to_guid {
        if let CombatEvent::GuidTeam(f) = evt {
            map.insert(i64::from(f.content_id), f.guid_hex());
        }
    }
    map
}

/// ObjectiveType 的 JSON 名（`GetObjectiveTypeName`；Unknown → "None"）。
fn type_name(t: &str) -> String {
    match t {
        "Camp" | "Tower" | "Ruins" | "Keep" | "Castle" => t.to_string(),
        _ => "None".to_string(),
    }
}

/// WvWHelper.cs `ObjectiveDataPerMapIDPerObjectiveID` 的 **type 投影**
///（EBG 22 / 三张 Alpine/Desert 各 18;EotM 等无表 → 全 Unknown → 过滤）。
/// 键 = map id → (objective id → type)。
fn objective_type(map_id: i32, objective_id: i32) -> Option<&'static str> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<std::collections::HashMap<(i32, i32), &'static str>> =
        OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        // map 96 Blue Alpine Borderlands
        for (id, ty) in [
            (39, "Camp"), (38, "Tower"), (37, "Keep"), (40, "Tower"), (52, "Camp"),
            (51, "Camp"), (64, "Ruins"), (65, "Ruins"), (63, "Ruins"), (66, "Ruins"),
            (62, "Ruins"), (33, "Keep"), (35, "Tower"), (53, "Camp"), (32, "Keep"),
            (36, "Tower"), (50, "Camp"), (34, "Camp"),
        ] {
            m.insert((96, id), ty);
        }
        // map 95 Green Alpine Borderlands
        for (id, ty) in [
            (39, "Camp"), (38, "Tower"), (37, "Keep"), (40, "Tower"), (52, "Camp"),
            (51, "Camp"), (64, "Ruins"), (65, "Ruins"), (63, "Ruins"), (66, "Ruins"),
            (62, "Ruins"), (33, "Keep"), (35, "Tower"), (53, "Camp"), (32, "Keep"),
            (36, "Tower"), (50, "Camp"), (34, "Camp"),
        ] {
            m.insert((95, id), ty);
        }
        // map 1099 Red Desert Borderlands
        for (id, ty) in [
            (99, "Camp"), (102, "Tower"), (113, "Keep"), (104, "Tower"), (115, "Camp"),
            (109, "Camp"), (122, "Ruins"), (119, "Ruins"), (120, "Ruins"), (121, "Ruins"),
            (118, "Ruins"), (106, "Keep"), (110, "Tower"), (101, "Camp"), (114, "Keep"),
            (105, "Tower"), (100, "Camp"), (116, "Camp"),
        ] {
            m.insert((1099, id), ty);
        }
        // map 38 Eternal Battlegrounds
        for (id, ty) in [
            (6, "Camp"), (17, "Tower"), (18, "Tower"), (1, "Keep"), (20, "Tower"),
            (19, "Tower"), (5, "Camp"), (8, "Camp"), (22, "Tower"), (21, "Tower"),
            (2, "Keep"), (15, "Tower"), (16, "Tower"), (7, "Camp"), (4, "Camp"),
            (13, "Tower"), (14, "Tower"), (3, "Keep"), (11, "Tower"), (12, "Tower"),
            (10, "Camp"), (9, "Castle"),
        ] {
            m.insert((38, id), ty);
        }
        m
    });
    t.get(&(map_id, objective_id)).copied()
}

/// wvWMapData 块 + teamMap 副作用（JsonLogBuilder.cs:308-312 +
/// JsonWvWMapDataBuilder.cs）。wvw_teams 非空才触发；objective 按事件首现序、
/// 表外 (map, objective) 丢弃（= C# 构造时 IsUnknown 整行丢弃的等价后置过滤）；
/// owners = 行 (TeamID, Time) 原样对。
pub fn build_wvw_map_data(
    log: &gw2ei_model::ParsedLog,
    teams: &crate::ctx::meta::WvwTeamsSig,
    team_ids: &mut std::collections::BTreeSet<i64>,
) -> WvwMapData {
    for t in [teams.red_team, teams.blue_team, teams.green_team] {
        team_ids.insert(i64::from(t));
    }
    let objectives: Vec<WvwObjectiveData> = log
        .metadata
        .wvw_objective_statuses
        .iter()
        .filter_map(|o: &WvWObjectiveStatusEventFields| {
            let ty = objective_type(o.map_id, o.objective_id)?;
            Some(WvwObjectiveData {
                map_id: i64::from(o.map_id),
                objective_id: i64::from(o.objective_id),
                objective_type: type_name(ty),
                owners: o
                    .owners
                    .iter()
                    .map(|&(team, time)| vec![i64::from(team), time])
                    .collect(),
            })
        })
        .collect();
    WvwMapData {
        red_shard_id: i64::from(teams.red_shard),
        blue_shard_id: i64::from(teams.blue_shard),
        green_shard_id: i64::from(teams.green_shard),
        red_team_id: i64::from(teams.red_team),
        blue_team_id: i64::from(teams.blue_team),
        green_team_id: i64::from(teams.green_team),
        objective_data: objectives,
    }
}
