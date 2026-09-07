//! Actor 行视图:自建行级索引(一次遍历 events),把 C# `CombatData` 的
//! per-agent 分桶 + `Actor`/`SingleActor` 的 Get*Events(target,start,end)
//! 过滤语义表达为 Rust 查询。

use std::collections::BTreeMap;

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, AgentTable, DmgRow, NO_AGENT, ParsedLog};

/// 行级解析(两端 AgentId;事件持有地址)。
#[derive(Debug, Clone, Copy)]
pub struct DmgLine {
    pub ev: usize,
    pub time: i64,
    pub from: AgentId,
    pub to: AgentId,
}

/// 健康伤害行(直伤/非直伤/NoDamage;与 EventIndex.damage_from 同成员)。
#[derive(Debug, Clone, Copy)]
pub struct HealthLine {
    pub line: DmgLine,
    pub to_friendly: bool,
    pub has_hit: bool,
}

/// 行索引(C# CombatData 的伤害/破蔑/控场分桶 + Actor 时间过滤)。
#[derive(Debug, Clone, Default)]
pub struct RowIndex {
    pub health_by_from: BTreeMap<AgentId, Vec<HealthLine>>,
    pub health_by_to: BTreeMap<AgentId, Vec<HealthLine>>,
    pub breakbar_by_from: BTreeMap<AgentId, Vec<DmgLine>>,
    pub breakbar_by_to: BTreeMap<AgentId, Vec<DmgLine>>,
    pub cc_by_from: BTreeMap<AgentId, Vec<DmgLine>>,
    pub cc_by_to: BTreeMap<AgentId, Vec<DmgLine>>,
}

impl RowIndex {
    pub fn new(log: &ParsedLog) -> Self {
        let mut idx = RowIndex::default();
        let mut resolver = log.agents.clone();
        for (i, evt) in log.events.iter().enumerate() {
            let time = evt.time().unwrap_or(0);
            let (a, b) = evt.participants();
            let from = resolve(&mut resolver, a, time);
            let to = resolve(&mut resolver, b, time);
            let line = DmgLine { ev: i, time, from, to };
            match evt {
                CombatEvent::DirectHealthDamage(f)
                | CombatEvent::NonDirectHealthDamage(f)
                | CombatEvent::NoDamageHealthDamage(f) => {
                    let hl = HealthLine {
                        line,
                        to_friendly: f.skill.iff == gw2ei_parse::Iff::Friend,
                        has_hit: f.has_hit,
                    };
                    push(&mut idx.health_by_from, hl.line.from, hl);
                    push(&mut idx.health_by_to, hl.line.to, hl);
                }
                CombatEvent::BreakbarDamage(_) | CombatEvent::BreakbarRecovery(_) => {
                    push(&mut idx.breakbar_by_from, line.from, line);
                    push(&mut idx.breakbar_by_to, line.to, line);
                }
                CombatEvent::CrowdControl(_) => {
                    push(&mut idx.cc_by_from, line.from, line);
                    push(&mut idx.cc_by_to, line.to, line);
                }
                _ => {}
            }
        }
        idx
    }
}

fn resolve(table: &mut AgentTable, addr: u64, time: i64) -> AgentId {
    if addr == 0 {
        return NO_AGENT;
    }
    let id = table.resolve_agent(addr, time);
    if id != NO_AGENT {
        return id;
    }
    // 事件时刻表未覆盖(如 metadata 无时间):按地址全表匹配
    table
        .all_ids()
        .into_iter()
        .find(|&x| table.slot(x).expect("slot").address == addr)
        .unwrap_or(NO_AGENT)
}

fn push<T: Copy>(map: &mut BTreeMap<AgentId, Vec<T>>, k: AgentId, v: T) {
    if k != NO_AGENT {
        map.entry(k).or_default().push(v);
    }
}

/// actor 查询面(C# SingleActor):root 含 englobed 段;伤害侧含 minion 链。
pub struct ActorRows<'a> {
    pub log: &'a ParsedLog,
    pub idx: &'a RowIndex,
    /// 承伤/自身伤害角色:root + englobed 段。
    pub identity: Vec<AgentId>,
    /// 伤害角色:identity + master 链终结于 root 的 NPC(minions)。
    pub damage_agents: Vec<AgentId>,
    pub root: AgentId,
}

impl<'a> ActorRows<'a> {
    pub fn new(log: &'a ParsedLog, idx: &'a RowIndex, root: AgentId) -> Self {
        let table = &log.agents;
        let mut identity = vec![root];
        if let Some(a) = table.slot(root) {
            identity.extend(a.englobed.iter().copied());
        }
        let root_root = table.englobing_root(root);
        let mut damage_agents = identity.clone();
        for id in table.all_ids() {
            if identity.contains(&id) {
                continue;
            }
            let a = table.slot(id).expect("slot");
            if (a.agent_type == gw2ei_model::AgentType::StableSpecies
                || a.agent_type == gw2ei_model::AgentType::VolatileSpecies)
                && table.final_master(id) == root_root
            {
                damage_agents.push(id);
            }
        }
        ActorRows { log, idx, identity, damage_agents, root }
    }

    /// 桶行 + 时间窗口过滤。
    fn window<T: Copy>(rows: &[T], start: i64, end: i64, f: impl Fn(&T) -> i64) -> Vec<T> {
        rows.iter().copied().filter(|r| {
            let t = f(r);
            start <= t && t <= end
        }).collect()
    }

    /// C# GetDamageEvents(target):全行 from ∈ damage_agents && !ToFriendly;
    /// target 过滤 to identity == target.Englobing(+其 aware 窗口)。
    pub fn health_out(&self, start: i64, end: i64, target: Option<AgentId>) -> Vec<HealthLine> {
        self.health_out_agents(&self.damage_agents, start, end, target)
    }

    /// C# GetJustActorDamageEvents(target):仅 from ∈ identity(无 minion;
    /// JsonActorBuilder 的 totalDamageDist/targetDamageDist 行源)。
    pub fn health_out_just(&self, start: i64, end: i64, target: Option<AgentId>) -> Vec<HealthLine> {
        self.health_out_agents(&self.identity, start, end, target)
    }

    fn health_out_agents(
        &self,
        froms: &[AgentId],
        start: i64,
        end: i64,
        target: Option<AgentId>,
    ) -> Vec<HealthLine> {
        let table = &self.log.agents;
        let mut out = Vec::new();
        for &from in froms {
            if let Some(rows) = self.idx.health_by_from.get(&from) {
                for hl in Self::window(rows, start, end, |r| r.line.time) {
                    if hl.to_friendly {
                        continue;
                    }
                    if let Some(t) = target {
                        let Some(ta) = table.slot(t) else { continue };
                        let t_root = table.englobing_root(t);
                        let Some(_to_item) = table.slot(hl.line.to) else { continue };
                        if table.englobing_root(hl.line.to) != t_root {
                            continue;
                        }
                        if !(ta.first_aware <= hl.line.time && hl.line.time <= ta.last_aware) {
                            continue;
                        }
                    }
                    out.push(hl);
                }
            }
        }
        out.sort_by_key(|l| l.line.ev);
        out
    }

    /// C# GetDamageTakenEvents(target):to ∈ identity;target 过滤 from。
    pub fn health_in(&self, start: i64, end: i64, target: Option<AgentId>) -> Vec<HealthLine> {
        let table = &self.log.agents;
        let mut out = Vec::new();
        for &to in &self.identity {
            if let Some(rows) = self.idx.health_by_to.get(&to) {
                for hl in Self::window(rows, start, end, |r| r.line.time) {
                    if let Some(t) = target {
                        let Some(ta) = table.slot(t) else { continue };
                        if table.englobing_root(hl.line.from) != table.englobing_root(t) {
                            continue;
                        }
                        if !(ta.first_aware <= hl.line.time && hl.line.time <= ta.last_aware) {
                            continue;
                        }
                    }
                    out.push(hl);
                }
            }
        }
        out.sort_by_key(|l| l.line.ev);
        out
    }

    fn bucket_out(
        map: &BTreeMap<AgentId, Vec<DmgLine>>,
        agents: &[AgentId],
        start: i64,
        end: i64,
    ) -> Vec<DmgLine> {
        let mut out = Vec::new();
        for &a in agents {
            if let Some(rows) = map.get(&a) {
                out.extend(Self::window(rows, start, end, |r| r.time));
            }
        }
        out.sort_by_key(|l| l.ev);
        out
    }

    pub fn breakbar_out(&self, start: i64, end: i64) -> Vec<DmgLine> {
        Self::bucket_out(&self.idx.breakbar_by_from, &self.damage_agents, start, end)
    }
    /// 仅 actor 自身(无 minion;JsonActorBuilder 的 dist breakbar 行源)。
    pub fn breakbar_out_just(&self, start: i64, end: i64) -> Vec<DmgLine> {
        Self::bucket_out(&self.idx.breakbar_by_from, &self.identity, start, end)
    }
    pub fn breakbar_in(&self, start: i64, end: i64) -> Vec<DmgLine> {
        Self::bucket_out(&self.idx.breakbar_by_to, &self.identity, start, end)
    }
    pub fn cc_out(&self, start: i64, end: i64) -> Vec<DmgLine> {
        Self::bucket_out(&self.idx.cc_by_from, &self.damage_agents, start, end)
    }
    pub fn cc_in(&self, start: i64, end: i64) -> Vec<DmgLine> {
        Self::bucket_out(&self.idx.cc_by_to, &self.identity, start, end)
    }

    /// 行 → DmgRow(分类轴在调用方注入;to 侧行 from/to 补全)。
    pub fn dmg_row(&self, line: &DmgLine) -> Option<DmgRow> {
        gw2ei_model::dmg_row(&self.log.events[line.ev], line.from, line.to)
    }

    /// 行 → breakbar 值(BreakbarDamage.value;C# `BreakbarDamage` 1 位小数
    /// round 由调用方)。
    pub fn breakbar_value(&self, line: &DmgLine) -> f64 {
        match &self.log.events[line.ev] {
            CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => f.value,
            _ => 0.0,
        }
    }
    pub fn cc_duration(&self, line: &DmgLine) -> i64 {
        match &self.log.events[line.ev] {
            CombatEvent::CrowdControl(f) => i64::from(f.duration),
            _ => 0,
        }
    }
}

// ===== 补充行索引(cast / weapon swap / buff 面)=====

/// BuffApply 行(C# BuffApplyEvent;to 侧)。
#[derive(Debug, Clone, Copy)]
pub struct BuffApplyLine {
    pub ev: usize,
    pub time: i64,
    pub buff_id: i64,
    pub applied_duration: i32,
    pub to: AgentId,
}

/// BuffRemove 行(C# BuffRemoveAllEvent;by 方向已修正)。kind 区分事件
/// 变体:0=RemoveAll、1=RemoveSingle、2=RemoveManual —— C# 的 strip/cleanse
/// 统计只取 RemoveAll(GetBuffRemoveAllEvents 系)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffRemoveAllLine {
    pub ev: usize,
    pub time: i64,
    pub buff_id: i64,
    pub removed_duration: i32,
    pub to: AgentId,
    pub by: AgentId,
    /// brae.ToFriendly(IFF 依据 To 与 By?C# = `ToFriendly`(事件 iff 对
    /// to 的描述)——BuffRemoveAll 由 CombatItem 的 IFF 字段承载(To 侧)。
    pub to_friendly: bool,
    pub to_foe: bool,
    pub kind: u8,
}

/// Movement 行种类（P4 CR 采样输入;C# MovementEvent 子类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    Position,
    Teleport,
    Velocity,
    Rotation,
}

/// Movement 行（坐标解包后的原始值;消费端过滤语义在 replay.rs —— C#
/// PositionEvent/TeleportEvent 的 AddPoint3D 内 Drop 规则）。
#[derive(Debug, Clone, Copy)]
pub struct MoveLine {
    pub ev: usize,
    pub time: i64,
    pub kind: MoveKind,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// cast/weapon/buff 桶(补充 RowIndex;遍历同一 events)。
#[derive(Debug, Clone, Default)]
pub struct AuxIndex {
    pub cast_by_caster: BTreeMap<AgentId, Vec<usize>>,
    pub weapon_swap_by_caster: BTreeMap<AgentId, Vec<usize>>,
    pub buff_apply_to: BTreeMap<AgentId, Vec<BuffApplyLine>>,
    pub buff_remove_all_by_to: BTreeMap<AgentId, Vec<BuffRemoveAllLine>>,
    /// 按 by(移除者)侧;strip/cleanse 统计(C# GetBuffRemoveAllEventsByByID)。
    pub buff_remove_all_by_from: BTreeMap<AgentId, Vec<BuffRemoveAllLine>>,
    /// Downed buff apply(玩家 down 计数;与 buff_apply_to 冗余但独立可查)。
    pub dead_rows: BTreeMap<AgentId, Vec<usize>>,
    pub down_rows: BTreeMap<AgentId, Vec<usize>>,
    pub up_rows: BTreeMap<AgentId, Vec<usize>>,
    pub spawn_rows: BTreeMap<AgentId, Vec<usize>>,
    pub despawn_rows: BTreeMap<AgentId, Vec<usize>>,
    pub enter_rows: BTreeMap<AgentId, Vec<usize>>,
    pub exit_rows: BTreeMap<AgentId, Vec<usize>>,
    pub health_update_rows: BTreeMap<AgentId, Vec<usize>>,
    pub barrier_update_rows: BTreeMap<AgentId, Vec<usize>>,
    pub max_health_rows: BTreeMap<AgentId, Vec<usize>>,
    /// movement 行(Position/Rotation/Velocity/Teleport;P4 combatReplay 输入)。
    pub movement_rows: BTreeMap<AgentId, Vec<MoveLine>>,
}

impl AuxIndex {
    pub fn new(log: &ParsedLog) -> Self {
        let mut idx = AuxIndex::default();
        let mut resolver = log.agents.clone();
        for (i, evt) in log.events.iter().enumerate() {
            let time = evt.time().unwrap_or(0);
            let (a, b) = evt.participants();
            let from = resolve(&mut resolver, a, time);
            let to = resolve(&mut resolver, b, time);
            match evt {
                CombatEvent::AnimatedCast(_) | CombatEvent::Emote(_)
                | CombatEvent::GadgetInteract(_) | CombatEvent::BundlePickUp(_) => {
                    push(&mut idx.cast_by_caster, from, i);
                }
                CombatEvent::WeaponSwap(_) => {
                    push(&mut idx.weapon_swap_by_caster, from, i);
                }
                CombatEvent::BuffApply(f) => {
                    let l = BuffApplyLine {
                        ev: i,
                        time: f.apply.base.time,
                        buff_id: crate::signed_id(f.apply.base.buff_id),
                        applied_duration: f.applied_duration,
                        to,
                    };
                    push(&mut idx.buff_apply_to, to, l);
                }
                CombatEvent::BuffRemoveAll(f) => {
                    let l = BuffRemoveAllLine {
                        ev: i,
                        time: f.remove.base.time,
                        buff_id: crate::signed_id(f.remove.base.buff_id),
                        removed_duration: f.remove.removed_duration,
                        to,
                        by: from,
                        to_friendly: f.remove.base.iff == gw2ei_parse::Iff::Friend,
                        to_foe: f.remove.base.iff == gw2ei_parse::Iff::Foe,
                        kind: 0,
                    };
                    push(&mut idx.buff_remove_all_by_to, to, l);
                    push(&mut idx.buff_remove_all_by_from, from, l);
                }
                // Single/Manual 与 All 同构(removed_duration=Value 近似;
                // buff 出现收集只需要 buff_id/to)。
                CombatEvent::BuffRemoveSingle(f) => {
                    let l = BuffRemoveAllLine {
                        ev: i,
                        time: f.remove.base.time,
                        buff_id: crate::signed_id(f.remove.base.buff_id),
                        removed_duration: f.remove.removed_duration,
                        to,
                        by: from,
                        to_friendly: f.remove.base.iff == gw2ei_parse::Iff::Friend,
                        to_foe: f.remove.base.iff == gw2ei_parse::Iff::Foe,
                        kind: 1,
                    };
                    push(&mut idx.buff_remove_all_by_to, to, l);
                    push(&mut idx.buff_remove_all_by_from, from, l);
                }
                CombatEvent::BuffRemoveManual(f) => {
                    let l = BuffRemoveAllLine {
                        ev: i,
                        time: f.base.time,
                        buff_id: crate::signed_id(f.base.buff_id),
                        removed_duration: f.removed_duration,
                        to,
                        by: from,
                        to_friendly: f.base.iff == gw2ei_parse::Iff::Friend,
                        to_foe: f.base.iff == gw2ei_parse::Iff::Foe,
                        kind: 2,
                    };
                    push(&mut idx.buff_remove_all_by_to, to, l);
                    push(&mut idx.buff_remove_all_by_from, from, l);
                }
                CombatEvent::Dead(_) => push(&mut idx.dead_rows, from, i),
                CombatEvent::Down(_) => push(&mut idx.down_rows, from, i),
                CombatEvent::Alive(_) => push(&mut idx.up_rows, from, i),
                CombatEvent::Spawn(_) => push(&mut idx.spawn_rows, from, i),
                CombatEvent::Despawn(_) => push(&mut idx.despawn_rows, from, i),
                CombatEvent::EnterCombat(_) => push(&mut idx.enter_rows, from, i),
                CombatEvent::ExitCombat(_) => push(&mut idx.exit_rows, from, i),
                CombatEvent::HealthUpdate(_) => push(&mut idx.health_update_rows, from, i),
                CombatEvent::BarrierUpdate(_) => push(&mut idx.barrier_update_rows, from, i),
                CombatEvent::MaxHealthUpdate(_) => push(&mut idx.max_health_rows, from, i),
                CombatEvent::Position(f) => push_move(&mut idx.movement_rows, MoveLine {
                    ev: i,
                    time: f.base.time,
                    kind: MoveKind::Position,
                    x: f.x,
                    y: f.y,
                    z: f.z,
                }, from),
                CombatEvent::Teleport(f) => push_move(&mut idx.movement_rows, MoveLine {
                    ev: i,
                    time: f.base.time,
                    kind: MoveKind::Teleport,
                    x: f.x,
                    y: f.y,
                    z: f.z,
                }, from),
                CombatEvent::Velocity(f) => push_move(&mut idx.movement_rows, MoveLine {
                    ev: i,
                    time: f.base.time,
                    kind: MoveKind::Velocity,
                    x: f.x,
                    y: f.y,
                    z: f.z,
                }, from),
                CombatEvent::Rotation(f) => push_move(&mut idx.movement_rows, MoveLine {
                    ev: i,
                    time: f.base.time,
                    kind: MoveKind::Rotation,
                    x: f.x,
                    y: f.y,
                    z: f.z,
                }, from),
                _ => {}
            }
        }
        idx
    }
}

fn push_move(map: &mut BTreeMap<AgentId, Vec<MoveLine>>, l: MoveLine, k: AgentId) {
    if k != NO_AGENT {
        map.entry(k).or_default().push(l);
    }
}
