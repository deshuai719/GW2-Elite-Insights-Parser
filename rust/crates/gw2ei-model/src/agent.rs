//! AgentItem / AgentTable：agent 链接层（P2a）。
//!
//! 对齐 C# `ParsedData/Agents/{AgentItem,AgentData}.cs` + `EvtcParser.cs`
//! `CompleteAgentsAndLogData`（:1144-1360）的语义（快照 `3b7278f9b`）。
//!
//! 设计：
//! - AgentItem 存放在 `AgentTable` 的 `Vec<Option<AgentItem>>` 槽位里，槽位号即
//!   稳定 `AgentId`（C# 引用语义的等价物：对象可移动、引用（id）不失效；
//!   删除 = `Option::take`，不重排）。
//! - `NO_AGENT` = C# `ParserHelper._unknownAgent`（地址解析失败 / 地址 0）。
//! - 事件与 agent 的关联按 `(address, time)` 解析（`resolve_agent`）：同地址
//!   多 agent（重定向归并 + spec 切分产生的 englobed 段）按 `FirstAware`
//!   稳定升序取首个覆盖 `time` 者 —— 对齐 `AgentData.GetAgent`
//!   （AgentData.cs:91-111）。
//! - 归并（RegroupSameAgentsAndDetermineTeams）与分裂
//!   （SplitPlayerPerSpecSubgroupAndSwap）在 `manipulation.rs`；本文件只放
//!   模型与查询面。

use std::collections::BTreeMap;

use crate::spec::{Spec, SpecCatalog};

/// 触发/物种 id 常量（SpeciesIDs.cs 子集）。
pub mod species {
    /// `TargetID.NonIdentifiedSpecies`（未知物种的默认 ID）。
    pub const NON_IDENTIFIED: i32 = 0;
    /// `TargetID.WorldVersusWorld`（WvW 伪目标 "Enemy Players"）。
    pub const WORLD_VERSUS_WORLD: i32 = 1;
    /// `TargetID.Instance`（instance 日志的触发 id）。
    pub const INSTANCE: i32 = 2;
    /// `TargetID.Environment`（fake Environment agent 的物种）。
    pub const ENVIRONMENT: i32 = -36;
    /// `TargetID.DummyTarget`。
    pub const DUMMY_TARGET: i32 = -1;
}

/// 稳定 agent 槽位 id。`NO_AGENT` 表示「无 / unknown」（C#
/// `ParserHelper._unknownAgent`，IsUnknown=true、IsPlayer=false）。
pub type AgentId = u32;
pub const NO_AGENT: AgentId = u32::MAX;

/// `AgentItem.AgentType`（AgentItem.cs:45）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentType {
    StableSpecies,
    VolatileSpecies,
    Player,
    NonSquadPlayer,
}

impl AgentType {
    /// `AgentItem.IsPlayer`（AgentItem.cs:47）。
    pub fn is_player(self) -> bool {
        matches!(self, AgentType::Player | AgentType::NonSquadPlayer)
    }
    pub fn is_npc(self) -> bool {
        !self.is_player()
    }
}

/// 归并来源快照（RegroupAgents/Redirect* 中被合并的 agent 可能已从表删除；
/// C# 对象仍存活可读，Rust 以快照保留后续 Split 的 `couple.from` 选择所需
/// 字段 —— AgentManipulationHelper.cs:224 的 `Regrouped.LastOrNull(...)`）。
#[derive(Debug, Clone)]
pub struct AgentMergedFrom {
    pub first_aware: i64,
    pub last_aware: i64,
    pub name: String,
    pub character: String,
    pub spec: Spec,
    pub base_spec: Spec,
    pub agent_type: AgentType,
    pub toughness: u16,
    pub healing: u16,
    pub condition: u16,
    pub concentration: u16,
    pub is_fake: bool,
}

/// `AgentItem`（AgentItem.cs:52-...）字段面。`name` 保留完整 `\0` 分段原始串
/// （68B lossy 解码，对齐 C# `GetString(68, nullTerminated:false)`）；
/// `character`/`account`/`group` 在构造时按 C# 拆段语义填充
/// （`Actor`/`Player` 构造器所用；NonSquadPlayer 之后由 `PlayerNonSquad`
/// 覆写，见 actors.rs）。
#[derive(Debug, Clone)]
pub struct AgentItem {
    pub address: u64,
    pub name: String,
    pub agent_type: AgentType,
    pub spec: Spec,
    pub base_spec: Spec,
    /// 物种 id（NPC/Gadget 的 prof 截断；玩家恒 0 = NonIdentifiedSpecies）。
    pub id: i32,
    pub instid: u16,
    pub toughness: u16,
    pub healing: u16,
    pub condition: u16,
    pub concentration: u16,
    pub hitbox_width: u32,
    pub hitbox_height: u32,
    /// 首次/最后 aware 时间。`i64::MAX` = 从未被事件覆盖（C# `long.MaxValue`）。
    pub first_aware: i64,
    pub last_aware: i64,
    /// master（minion 归属链；`NO_AGENT` = 无）。SetMaster 存 `EnglobingAgentItem`。
    pub master: AgentId,
    /// englobing 根（spec/子群切分后的父 agent；`NO_AGENT` = 自身即根）。
    pub englobing: AgentId,
    /// englobed 子段（`SetEnglobingAgentItem` 反向登记）。
    pub englobed: Vec<AgentId>,
    /// 被归并来源（RegroupAgents；合并后的事件已重写地址）。
    pub regrouped: Vec<AgentMergedFrom>,
    pub is_fake: bool,
    pub is_not_in_squad_friendly_player: bool,
    pub is_unknown: bool,
    /// 角色名（agent.Name 拆 `\0` 首段）。
    pub character: String,
    /// 账号（玩家名第 2 段去前导 `:`；非玩家 None）。
    pub account: Option<String>,
    /// 子群（玩家名第 3 段 int；非玩家 None）。0 = 未入小队。
    pub group: Option<i32>,
    /// C# `AgentItem.Unamed`：名字含 "ch{id}-"/"gd{id}-"。
    pub unamed: bool,
}

impl AgentItem {
    pub fn half_aware(&self) -> i64 {
        (self.first_aware + self.last_aware) / 2
    }

    pub fn in_aware_times(&self, time: i64) -> bool {
        self.first_aware <= time && time <= self.last_aware
    }

    /// `InAwareTimes(start, end)`：段相交（Segment.Intersects，闭区间）。
    pub fn aware_intersects(&self, start: i64, end: i64) -> bool {
        self.first_aware <= end && self.last_aware >= start
    }

    /// `CouldBeEqual`（AgentItem.cs:152-158）：同 instid/物种/类型/名字/
    /// master（注：C# 注释掉了 hitbox 比较）。
    pub fn could_be_equal(&self, other: &AgentItem) -> bool {
        self.instid == other.instid
            && self.id == other.id
            && self.agent_type == other.agent_type
            && self.unamed == other.unamed
            && self.name == other.name
            && self.master == other.master
    }

    /// `IsNonIdentifiedSpecies`（AgentItem.cs:544-551）：玩家恒 false；其余
    /// unknown 或物种 ∈ {0, 1, -36, 2, -1}。
    pub fn is_non_identified_species(&self) -> bool {
        if self.agent_type.is_player() {
            return false;
        }
        self.is_unknown
            || matches!(
                self.id,
                species::NON_IDENTIFIED
                    | species::WORLD_VERSUS_WORLD
                    | species::ENVIRONMENT
                    | species::INSTANCE
                    | species::DUMMY_TARGET
            )
    }
}

/// agent 槽位表。槽位号 = `AgentId`；`Option::None` = 已删除（C#
/// `_allAgentsList` 删除语义）。
#[derive(Debug, Clone, Default)]
pub struct AgentTable {
    slots: Vec<Option<AgentItem>>,
    /// `(address → 槽位)`：`refresh_by_address` 时重建，槽位按
    /// `(first_aware, 创建序)` 稳定升序。
    by_address: BTreeMap<u64, Vec<AgentId>>,
    /// 每种类型的槽位缓存（脏标记语义：C# `_dirty` + Refresh）。
    by_type_cache: Option<BTreeMap<AgentType, Vec<AgentId>>>,
    /// 按 instid 的缓存（仅链接后查询用，重建成本低）。
    by_instid_cache: Option<BTreeMap<u16, Vec<AgentId>>>,
}

impl AgentTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn slot(&self, id: AgentId) -> Option<&AgentItem> {
        self.slots.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn slot_mut(&mut self, id: AgentId) -> Option<&mut AgentItem> {
        self.slots.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// 追加新 agent（构造后必须 `refresh` 才能被查询面看到）。
    pub fn push_agent(&mut self, mut agent: AgentItem) -> AgentId {
        let id = self.slots.len() as AgentId;
        agent.is_unknown = false;
        self.slots.push(Some(agent));
        self.invalidate_caches();
        id
    }

    /// 删除（C# `_allAgentsList.RemoveAll` / `RemoveAllFrom`）。
    pub fn remove_agent(&mut self, id: AgentId) {
        if let Some(slot) = self.slots.get_mut(id as usize) {
            *slot = None;
        }
        self.invalidate_caches();
    }

    fn invalidate_caches(&mut self) {
        self.by_address.clear();
        self.by_type_cache = None;
        self.by_instid_cache = None;
    }

    /// 遍历所有存活槽位（稳定序 = 槽位序；调用方按需排序）。
    pub fn iter(&self) -> impl Iterator<Item = AgentId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|_| i as AgentId))
    }

    /// `_allAgentsList.SortByFirstAware` 语义：返回按 `(first_aware, 槽位序)`
    /// 稳定排序的 id 列表。
    pub fn ids_sorted_by_first_aware(&self) -> Vec<AgentId> {
        let mut ids: Vec<AgentId> = self.iter().collect();
        ids.sort_by_key(|&id| (self.slot(id).expect("slot").first_aware, id));
        ids
    }

    /// 重建按地址索引（同地址多 agent 按 first_aware 升序；GetAgent 遍历序）。
    fn refresh_by_address(&mut self) {
        if !self.by_address.is_empty() || self.is_empty() {
            return;
        }
        let mut groups: BTreeMap<u64, Vec<AgentId>> = BTreeMap::new();
        for id in self.ids_sorted_by_first_aware() {
            groups
                .entry(self.slot(id).expect("slot").address)
                .or_default()
                .push(id);
        }
        self.by_address = groups;
    }

    fn refresh_type_cache(&mut self) {
        if self.by_type_cache.is_some() {
            return;
        }
        let mut map: BTreeMap<AgentType, Vec<AgentId>> = BTreeMap::new();
        // C# Refresh：GetAgentByType 排除 englobing 根？—— 不：C# `_allAgentsByType`
        // 由 `notEnglobingAgents` 建 —— 只含**非 englobing**（=根）agent。
        for id in self.ids_sorted_by_first_aware() {
            let a = self.slot(id).expect("slot");
            if a.englobing == NO_AGENT {
                map.entry(a.agent_type).or_default().push(id);
            }
        }
        self.by_type_cache = Some(map);
    }

    fn refresh_instid_cache(&mut self) {
        if self.by_instid_cache.is_some() {
            return;
        }
        let mut map: BTreeMap<u16, Vec<AgentId>> = BTreeMap::new();
        for id in self.ids_sorted_by_first_aware() {
            let a = self.slot(id).expect("slot");
            map.entry(a.instid).or_default().push(id);
        }
        self.by_instid_cache = Some(map);
    }

    /// `GetAgentByType`（AgentData.cs:279-290）：非 englobing 的该类型 agent，
    /// 按 FirstAware 升序。
    pub fn agents_by_type(&mut self, ty: AgentType) -> Vec<AgentId> {
        self.refresh_type_cache();
        self.by_type_cache
            .as_ref()
            .and_then(|m| m.get(&ty).cloned())
            .unwrap_or_default()
    }

    /// `GetAgentByInstID(instid, time)`（AgentData.cs:203-223）。
    pub fn agent_by_instid(&mut self, instid: u16, time: i64) -> AgentId {
        if instid == 0 {
            return NO_AGENT;
        }
        self.refresh_instid_cache();
        if let Some(list) = self.by_instid_cache.as_ref().and_then(|m| m.get(&instid)) {
            for &id in list {
                let a = self.slot(id).expect("slot");
                if a.in_aware_times(time) {
                    return id;
                }
            }
        }
        NO_AGENT
    }

    /// `GetAgent(agentAddress, time)`（AgentData.cs:91-111）：同地址列表按
    /// FirstAware 序首个 aware 覆盖者；地址 0 / 无覆盖 → `NO_AGENT`。
    pub fn resolve_agent(&mut self, address: u64, time: i64) -> AgentId {
        if address == 0 {
            return NO_AGENT;
        }
        self.refresh_by_address();
        if let Some(list) = self.by_address.get(&address) {
            for &id in list {
                let a = self.slot(id).expect("slot");
                if a.in_aware_times(time) {
                    return id;
                }
            }
        }
        NO_AGENT
    }

    /// `EnglobingAgentItem`（AgentItem.cs:38）：根槽位 id（自身即根时返回自身）。
    pub fn englobing_root(&self, id: AgentId) -> AgentId {
        match self.slot(id) {
            Some(a) if a.englobing != NO_AGENT => a.englobing,
            _ => id,
        }
    }

    /// `Is(ag)` 的 id 版（englobing 根同一；NO_AGENT 相互不等）。
    pub fn same_identity(&self, a: AgentId, b: AgentId) -> bool {
        a != NO_AGENT && b != NO_AGENT && self.englobing_root(a) == self.englobing_root(b)
    }

    /// `GetFinalMaster`（id 版）。
    pub fn final_master(&self, id: AgentId) -> AgentId {
        let mut cur = id;
        let mut guard = 0;
        while let Some(a) = self.slot(cur) {
            if a.master == NO_AGENT {
                break;
            }
            cur = a.master;
            guard += 1;
            if guard > 1024 {
                break; // 防环（C# SetMaster 防环 + 本处兜底，不应达）
            }
        }
        cur
    }

    /// `SetMaster`（AgentItem.cs:238-254）：**minion 是玩家则拒绝**、
    /// 防环；存 `master.EnglobingAgentItem`。
    pub fn set_master(&mut self, minion: AgentId, master: AgentId) {
        let Some(minion_item) = self.slot(minion) else { return };
        if minion_item.agent_type.is_player() {
            return;
        }
        let master_root = self.englobing_root(master);
        if master_root == minion {
            return;
        }
        // 防环：沿 master 链上溯，若回到 minion 则拒绝
        let mut cur = master_root;
        loop {
            if cur == minion {
                return;
            }
            let Some(a) = self.slot(cur) else { break };
            if a.master == NO_AGENT {
                break;
            }
            cur = a.master;
        }
        if let Some(slot) = self.slot_mut(minion) {
            slot.master = master_root;
        }
    }

    /// `SetEnglobingAgentItem`（AgentItem.cs:624-628）：登记父子关系。
    pub fn set_englobing(&mut self, child: AgentId, parent: AgentId) {
        if let Some(c) = self.slot_mut(child) {
            c.englobing = parent;
        }
        if let Some(p) = self.slot_mut(parent)
            && !p.englobed.contains(&child) {
                p.englobed.push(child);
            }
    }

    /// `IsMasterOrSelf` / `IsMaster` 的 id 版（按 englobing identity）。
    pub fn is_master_or_self(&self, master: AgentId, other: AgentId) -> bool {
        self.same_identity(self.final_master(master), other)
    }

    /// 全部存活 id（任意序）。
    pub fn all_ids(&self) -> Vec<AgentId> {
        self.iter().collect()
    }

    /// `AgentData.GetAgentByType(Player)` 之后跟随的玩家身份规整函数集合：
    /// 本表构造完成后调用 `finalize` 一次性重建全部缓存。
    pub fn finalize(&mut self) {
        self.invalidate_caches();
        let _ = self.by_type_cache.take();
        self.refresh_by_address();
    }
}

// ===== 构造辅助 =====

/// 从 agent 区原始条目构造 AgentItem（对齐 ParseAgentData :544-587 +
/// AgentItem 构造器的玩家降级/名字拆段逻辑）。
pub fn agent_from_raw(
    raw: &gw2ei_parse::EvtcRawAgent,
    table: &mut AgentTable,
) -> AgentId {
    agent_from_raw_with(raw, table, &crate::spec::NoSpecCatalog)
}

/// `agent_from_raw` 的带 SpecList 版本（P2b；C# `GW2APIController.GetSpec`）。
pub fn agent_from_raw_with(
    raw: &gw2ei_parse::EvtcRawAgent,
    table: &mut AgentTable,
    catalog: &dyn SpecCatalog,
) -> AgentId {
    let name = raw.name_full_lossy();
    let spec = catalog.spec_of(raw.prof, raw.is_elite);
    let (id, agent_type) = match spec {
        Spec::Npc => (
            (if raw.prof > u16::MAX as u32 { 0 } else { raw.prof }) as i32,
            AgentType::StableSpecies,
        ),
        Spec::Gadget => ((raw.prof & 0xffff) as i32, AgentType::VolatileSpecies),
        _ => (species::NON_IDENTIFIED, AgentType::Player),
    };
    let (agent_type, character, account, group) = if agent_type == AgentType::Player {
        // AgentItem 构造器（AgentItem.cs:96-114）降级逻辑。C# 语义精确复刻：
        // 段数 <2 → NonSquad；`seg[1] 空 || seg[2] 空 || seg[0] 含 '-'` → NonSquad；
        // **段数 ==2 且 seg[1] 非空 → C# 求值 seg[2] 越界抛异常被吞 → 保持
        // Player**（本处显式复刻为 Player，注释锚点 AgentItem.cs:102-104）。
        let segs: Vec<&str> = name.split('\0').collect();
        let mut ty = agent_type;
        // 合并分支：C# 条件（AgentItem.cs:102-104）的短求值语义
        // `len<2 || seg[1]空 || (len>=3 && (seg[2]空 || seg0含'-'))`
        // —— len==2 && seg[1] 非空时 C# 对 seg[2] 越界抛异常被吞 → 保持 Player，
        // 与下方条件不触发等价（显式注释锚点，不静默）。
        if segs.len() < 2
            || segs[1].is_empty()
            || (segs.len() >= 3 && (segs[2].is_empty() || segs[0].contains('-')))
        {
            ty = AgentType::NonSquadPlayer;
        }
        // len==2 && seg[1] 非空 → C# 异常路径 → Player（上面条件不触发）。
        if ty == AgentType::Player {
            let character = segs[0].to_string();
            let account = segs[1].trim_start_matches(':').to_string();
            let group = segs.get(2).and_then(|s| s.parse::<i32>().ok());
            (ty, character, Some(account), group)
        } else {
            (ty, segs[0].to_string(), None, None)
        }
    } else {
        (agent_type, name.split('\0').next().unwrap_or("").to_string(), None, None)
    };
    let unamed = name.contains(&format!("ch{id}-")) || name.contains(&format!("gd{id}-"));
    table.push_agent(AgentItem {
        address: raw.address,
        name,
        agent_type,
        spec,
        base_spec: spec.base_spec(),
        id,
        instid: 0,
        toughness: raw.toughness,
        healing: raw.healing,
        condition: raw.condition,
        concentration: raw.concentration,
        // C# 读取时即 ×2（EvtcParser.cs:560/564）
        hitbox_width: u32::from(raw.hitbox_width_raw) * 2,
        hitbox_height: u32::from(raw.hitbox_height_raw) * 2,
        first_aware: i64::MAX,
        last_aware: i64::MAX,
        master: NO_AGENT,
        englobing: NO_AGENT,
        englobed: Vec::new(),
        regrouped: Vec::new(),
        is_fake: false,
        is_not_in_squad_friendly_player: false,
        is_unknown: false,
        character,
        account,
        group,
        unamed,
    })
}
