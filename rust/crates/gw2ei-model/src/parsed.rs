//! ParsedLog：P2a 组装层 —— LogData（WvW 路径）+ actor 列表 + 事件索引后的
//! 战斗数据（对应 C# `ParsedEvtcLog` + `LogData.cs` + `WvWLogic` 的最小面）。
//!
//! C# 流程（EvtcParser.cs:353-472 + ParsedEvtcLog.cs）：
//! ParseLog → CompleteAgentsAndLogData（link/regroup/split/players）→
//! PreProcessEvtcData（HandleCriticalAgents → OffsetEvtcData →
//! Logic.EIEvtcParse：WvW 建伪 target + 敌人 NonSquad 伤害重定向）→
//! ParsedEvtcLog ctor（CombatData 事件化）。本模块复刻这条链的 WvW 面：
//! 非 WvW（trigger != 1）→ `ModelError::UnsupportedEncounter`（45 boss 表 P4）。
//!
//! P2a 面向：`log_end` 由 ParseCombatList 的归零时间轴给出；OffsetEvtcData 对
//! WvW 恒为 0（`GetGenericLogOffset = EvtcLogStart = 0`，LogLogicTimeUtils.cs:26）；
//! Success 恒 true（WvWLogic.cs:301-304）→ LogEnd = min(logEnd+10, evtcEnd)。

use gw2ei_parse::read_evtc_log;

use crate::agent::{AgentId, AgentTable, AgentType, NO_AGENT, species};
use crate::error::ModelError;
use crate::events::CombatEvent;
use crate::manipulation::{LinkStats, complete_agents_with};
use crate::spec::{NoSpecCatalog, Spec, SpecCatalog};

/// WvW 模式日志的数据结构骨架（单主 phase "Full Fight"）。
#[derive(Debug, Clone)]
pub struct LogData {
    pub trigger_id: u16,
    pub log_start: i64,
    pub log_end: i64,
    pub evtc_log_end: i64,
    /// map id（首个 MapID 事件值；WvW 地图 id，-1 = 无）。
    pub map_id: i32,
    /// fightName（WvWLogic.GetLogicName 拼接，:155-…；不依赖 MapList 资产）。
    pub name: String,
    /// 主 phase（LogLogicPhaseUtils.cs:211-224 + WvWLogic.GetPhases）。
    pub main_phase: PhaseData,
    /// WvW 伪目标 agent（"Enemy Players" NPC，species WorldVersusWorld）。
    pub dummy_target_agent: AgentId,
}

/// 单 phase 的 C# `EncounterPhaseData` 最小面（PhaseData.cs）。
#[derive(Debug, Clone)]
pub struct PhaseData {
    pub start: i64,
    pub end: i64,
    pub name: String,
    pub targets: Vec<AgentId>,
}

/// Player actor（C# `Player`：SingleActor + group/account）。
#[derive(Debug, Clone)]
pub struct PlayerActor {
    pub agent: AgentId,
    pub account: String,
    pub character: String,
    pub group: i32,
    /// C# `_squadless`（MakeSquadless 置位 → JSON notInSquad）。
    pub squadless: bool,
}

/// PlayerNonSquad actor（PlayerNonSquad.cs:10-18）：Character =
/// "{Spec} pl-{InstID}"、Account = "Non Squad Player {n}"（静态计数）。
#[derive(Debug, Clone)]
pub struct NonSquadPlayerActor {
    pub agent: AgentId,
    pub account: String,
    pub character: String,
    pub group: i32,
}

/// NPC actor（C# `NPC`/`DummyActor` 面）。
#[derive(Debug, Clone)]
pub struct NpcActor {
    pub agent: AgentId,
    pub character: String,
    pub id: i32,
}

/// P2a 组装产物。`events` 等聚合面在 `stats.rs` 的 `EventIndex` 上实现。
#[derive(Debug)]
pub struct ParsedLog {
    /// 头 arc 版本与 ArcBuild 修订((header build, revision);C#
    /// EvtcVersionEvent:Build/Revision)。
    pub arc_version: (i32, i32),
    pub agents: AgentTable,
    pub link_stats: LinkStats,
    pub log_data: LogData,
    /// 玩家 actor（CompletePlayers 排序后：Character 序）。
    pub players: Vec<PlayerActor>,
    /// 友方非小队玩家（WvW：按 (spec, instid) 排序 —— WvWLogic.cs:338）。
    pub friendly_non_squad: Vec<NonSquadPlayerActor>,
    /// targets（非详 WvW = 仅伪 target）。
    pub targets: Vec<NpcActor>,
    /// 组合友好列表（players + friendly non-squad；C# Friendlies 语义）。
    pub friendlies: Vec<Friend>,
    /// 事件化后的行级事件（与 P1 相同,地址已按链接重写）。
    pub events: Vec<CombatEvent>,
    /// Metadata 分桶(单例/列表/字典行;P1 CombatData.metadata)。
    pub metadata: crate::combat_data::MetaDataBucket,
    /// skill 区原始 (id, 名)（evtc 客户端本地化名;json 层 skillMap 输入）。
    pub skills: Vec<(u32, String)>,
}

/// C# `log.Friendlies`（PlayerActor 集合；JSON players 块输入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Friend {
    Player(usize),
    NonSquad(usize),
}

/// 完整组装：读文件 → 链接 → 识别（WvW）→ 玩家 → 重定向 → 事件化。
/// 使用内置本地 spec 子集（无 SpecList）。
pub fn parse_log_file(path: &std::path::Path) -> Result<ParsedLog, ModelError> {
    let raw = read_evtc_log(path).map_err(|e| ModelError::ParseError(e.to_string()))?;
    assemble_raw(raw)
}

/// `parse_log_file` 的带 SpecList 版本（P2b）。
pub fn parse_log_file_with(
    path: &std::path::Path,
    catalog: &dyn SpecCatalog,
) -> Result<ParsedLog, ModelError> {
    let raw = read_evtc_log(path).map_err(|e| ModelError::ParseError(e.to_string()))?;
    assemble_raw_with(raw, catalog)
}

/// 由已解析 raw 组装（测试友好入口;本地 spec 子集）。
pub fn assemble_raw(raw: gw2ei_parse::EvtcRawLog) -> Result<ParsedLog, ModelError> {
    assemble_raw_with(raw, &NoSpecCatalog)
}

/// `assemble_raw` 的带 SpecList 版本。
pub fn assemble_raw_with(
    mut raw: gw2ei_parse::EvtcRawLog,
    catalog: &dyn SpecCatalog,
) -> Result<ParsedLog, ModelError> {
    let skills = raw.skills.iter().map(|sk| (sk.id, sk.name.clone())).collect::<Vec<_>>();
    let arc_version = raw.arc_version;
    // 头部快照（raw 随后被消费）
    let trigger_id = raw.id;
    let evtc_log_end = raw.log_end_time;
    let map_id = raw.map_id;
    let (mut agents, link_stats) = complete_agents_with(&mut raw, catalog)?;

    // ===== 识别（LogData.DetectLogic 的 WvW 面）=====
    // C#：trigger id → TargetID；1=WorldVersusWorld → WvWLogic（无 Desmina 时）。
    if trigger_id != species::WORLD_VERSUS_WORLD as u16 {
        return Err(ModelError::UnsupportedEncounter { trigger_id });
    }
    let name = wvw_fight_name(map_id);

    // ===== CompletePlayers（EvtcParser.cs:1037-1128）=====
    let mut player_agents = agents.agents_by_type(AgentType::Player);
    player_agents.retain(|&id| {
        let a = agents.slot(id).expect("slot");
        a.instid != 0 && a.last_aware != i64::MAX
    });
    if player_agents.is_empty() {
        return Err(ModelError::NoValidPlayers);
    }
    let mut players: Vec<PlayerActor> = Vec::with_capacity(player_agents.len());
    for id in player_agents {
        let a = agents.slot(id).expect("slot");
        players.push(PlayerActor {
            agent: id,
            account: a.account.clone().unwrap_or_default(),
            character: a.character.clone(),
            group: a.group.unwrap_or(0),
            squadless: false,
        });
    }
    if players.iter().any(|p| p.group == 0) {
        for p in &mut players {
            p.squadless = true;
            p.group = 1; // MakeSquadless
        }
    }
    // `OrderBy(Character)`（:1061）：.NET OrderBy 稳定排序。C# 用当前 culture
    // 的字符串比较；JSON 对拍以字节序为稳定约定 —— 记录：C# 侧为
    // Ordinal 时与字节序一致（.NET 默认 OrderBy 用 Comparer<string>.Default =
    // culture-aware?—— 实测 CultureInfo 无关的快速路径是 Ordinal；此处选
    // **Ordinal 字节序**，P2b 对拍若出现 culture 差异再按序数对齐）。
    players.sort_by(|a, b| a.character.cmp(&b.character));
    if players.is_empty() {
        return Err(ModelError::NoValidPlayers);
    }
    // toughness 归一 0-10（:1117-1126）：min>0 才执行；Math.Round = ToEven。
    let min_t = players.iter().map(|p| agents.slot(p.agent).expect("slot must exist").toughness).min().unwrap_or(0);
    if min_t > 0 {
        let max_t = players.iter().map(|p| agents.slot(p.agent).expect("slot must exist").toughness).max().unwrap_or(0);
        for p in &mut players {
            let t = agents.slot(p.agent).expect("slot must exist").toughness;
            let norm = (10.0 * f64::from(t - min_t) / f64::from((max_t - min_t).max(1))).round_ties_even() as u16;
            agents.slot_mut(p.agent).expect("slot_mut must exist").toughness = norm;
        }
    }

    // ===== WvWLogic.EIEvtcParse（WvWLogic.cs:306-378，非详模式）=====
    // 1) 伪 target "Enemy Players"（AddCustomNPCAgent [0, LogEnd], spec NPC,
    //    id=WorldVersusWorld, isFake）—— 随机地址/instid 语义同 C#。
    let dummy = add_fake_npc_agent(
        &mut agents,
        species::WORLD_VERSUS_WORLD,
        &Spec::Npc,
        "Enemy Players",
        evtc_log_end,
    );
    // 2) NonSquad player actor 壳（friendly/敌方分流）
    let non_squad_ids = agents.agents_by_type(AgentType::NonSquadPlayer);
    let mut friendly_non_squad: Vec<NonSquadPlayerActor> = Vec::new();
    let mut enemy_non_squad: Vec<AgentId> = Vec::new();
    let mut non_squad_counter = 0u32;
    for id in non_squad_ids {
        let a = agents.slot(id).expect("slot");
        let friendly = a.is_not_in_squad_friendly_player;
        non_squad_counter += 1;
        let actor = NonSquadPlayerActor {
            agent: id,
            account: format!("Non Squad Player {non_squad_counter}"),
            character: format!("{} pl-{}", a.spec.csharp_name(), a.instid),
            group: 51, // SingleActor ctor 默认 Group=51（SingleActor.cs:31）
        };
        if friendly {
            friendly_non_squad.push(actor);
        } else {
            enemy_non_squad.push(id);
        }
    }
    // C# 顺序：auxFriendlies.OrderBy(Spec).ThenBy(InstID)（WvWLogic.cs:338）
    friendly_non_squad.sort_by(|a, b| {
        let sa = agents.slot(a.agent).expect("slot must exist").spec;
        let sb = agents.slot(b.agent).expect("slot must exist").spec;
        (sa.csharp_name(), agents.slot(a.agent).expect("slot must exist").instid)
            .cmp(&(sb.csharp_name(), agents.slot(b.agent).expect("slot must exist").instid))
    });

    // 3) 非详模式：敌人伤害事件重定向到伪 target（WvWLogic.cs:340-372）：
    //    逐行判断 IsDamageEvent，src/dst 解析到 enemy non-squad → 改写。
    {
        let enemy: Vec<(u64, i64, i64)> = enemy_non_squad
            .iter()
            .map(|&id| {
                let a = agents.slot(id).expect("slot");
                (a.address, a.first_aware, a.last_aware)
            })
            .collect();
        for item in &mut raw.combat_events {
            if !is_damage_item(item) {
                continue;
            }
            for &(addr, fa, la) in &enemy {
                if item.src_agent == addr && fa <= item.time && item.time <= la {
                    item.src_agent = agents.slot(dummy).expect("dummy").address;
                    break;
                }
            }
            for &(addr, fa, la) in &enemy {
                if item.dst_agent == addr && fa <= item.time && item.time <= la {
                    item.dst_agent = agents.slot(dummy).expect("dummy").address;
                    break;
                }
            }
        }
    }

    // ===== 事件化（P1 factory,输入已重写/已删无效行）=====
    let combat_data = crate::factory::build_combat_data(raw)?;

    let targets = vec![NpcActor {
        agent: dummy,
        character: "Enemy Players".to_string(),
        id: species::WORLD_VERSUS_WORLD,
    }];
    let friendlies: Vec<Friend> = (0..players.len())
        .map(Friend::Player)
        .chain((0..friendly_non_squad.len()).map(Friend::NonSquad))
        .collect();

    let log_data = LogData {
        trigger_id,
        log_start: 0,
        log_end: evtc_log_end,
        evtc_log_end,
        map_id,
        name,
        main_phase: PhaseData {
            start: 0,
            end: evtc_log_end,
            name: "Full Fight".to_string(),
            targets: vec![dummy],
        },
        dummy_target_agent: dummy,
    };

    Ok(ParsedLog {
        agents,
        link_stats,
        log_data,
        players,
        friendly_non_squad,
        targets,
        friendlies,
        events: combat_data.events,
        metadata: combat_data.metadata,
        skills,
        arc_version,
    })
}

/// WvW fightName（WvWLogic.GetLogicName :155-…）："{default} - {地图名}"。
/// map id 常量取自 MapIDs 静态类（不依赖 MapList.json 资产）。
fn wvw_fight_name(map_id: i32) -> String {
    let default_name = "World vs World";
    let map = match map_id {
        // MapIDs.cs 子集（WvW 地图）
        38 => "Eternal Battlegrounds",
        95 => "Green Alpine Borderlands",
        96 => "Blue Alpine Borderlands",
        1099 => "Red Desert Borderlands",
        1100 => "Obsidian Sanctum",
        968 => "Edge of the Mists",
        1102 => "Armistice Bastion",
        _ => return default_name.to_string(),
    };
    format!("{default_name} - {map}")
}

/// AddCustomNPCAgent 的 fake NPC（随机地址/instid 唯一；spec/type 语义照抄）。
fn add_fake_npc_agent(
    agents: &mut AgentTable,
    id: i32,
    spec: &Spec,
    name: &str,
    log_end: i64,
) -> AgentId {
    use crate::agent::AgentItem;
    let start = 0i64;
    let end = log_end;
    // 生成唯一随机地址与 instid（C# AddCustomNPCAgent 的 Random 循环）
    let mut rng = 0x243f_6a88_85a3_08d3u64;
    let used_addr: Vec<u64> = agents
        .all_ids()
        .into_iter()
        .map(|i| agents.slot(i).expect("slot must exist").address)
        .collect();
    let addr = loop {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let c = (u64::from(u32::MAX) / 2) + (rng >> 33) % (u64::from(u32::MAX) / 2);
        if c != 0 && !used_addr.contains(&c) {
            break c;
        }
    };
    let used_inst: Vec<u16> = agents
        .all_ids()
        .into_iter()
        .map(|i| agents.slot(i).expect("slot must exist").instid)
        .collect();
    let instid = loop {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let c = (u16::MAX / 2) + (rng >> 48) as u16 % (u16::MAX / 2);
        if c != 0 && !used_inst.contains(&c) {
            break c;
        }
    };
    agents.push_agent(AgentItem {
        address: addr,
        name: name.to_string(),
        agent_type: AgentType::StableSpecies, // AddCustomNPCAgent 恒 StableSpecies
        spec: *spec,
        base_spec: spec.base_spec(),
        id,
        instid,
        toughness: 0,
        healing: 0,
        condition: 0,
        concentration: 0,
        hitbox_width: 0,
        hitbox_height: 0,
        first_aware: start,
        last_aware: end,
        master: NO_AGENT,
        englobing: NO_AGENT,
        englobed: Vec::new(),
        regrouped: Vec::new(),
        is_fake: true,
        is_not_in_squad_friendly_player: false,
        is_unknown: false,
        character: name.to_string(),
        account: None,
        group: None,
        unamed: false,
    })
}

/// 行级 IsDamageEvent（CombatItem.cs 新世代：Combat + IsBuff 区分；C#
/// `IsDamageEvent(extensions)` 等价于 direct|buff damage 判定）。
fn is_damage_item(item: &gw2ei_parse::EvtcCombatItem) -> bool {
    use gw2ei_parse::StateChange;
    item.state_change == StateChange::Combat
}
