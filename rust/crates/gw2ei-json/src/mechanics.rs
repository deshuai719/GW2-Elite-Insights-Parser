//! mechanics 块（P4）：commons + WvW 注册表的 14 个 mechanic（LogLogic.cs
//! :99-132 + WvWLogic.cs:46-56）的匹配与 JSON 构建。
//!
//! C# 语义（MechanicData.cs / JsonMechanicsBuilder.cs）：
//! - 注册序即输出序；无事件（GetPresentMechanics 判空）的整条剔除；
//! - PlayerStatus 系遍历 PlayerList（仅小队玩家）× 事件行；Res（cast
//!   1066）按 GetAnimatedCastData(1066) 全流 + TryGetActor(Caster) 玩家过滤；
//! - CC 六种门控 `HasCrowdControlData`（日志有 CC 事件才 Available）；
//!   PlayerDst 语义：按 skill id 取 CC 行、To（受击方）为小队玩家才计入；
//! - KillingBlow 双 checker：Player 侧 To ∈ {NonSquadPlayer, WvW 物种}；
//!   Enemy 侧（Hostiles = [伪 target] 的承伤行）CreditedFrom 终主为
//!   Player 类型（非小队玩家不算 —— C# AgentType.Player 精确匹配）。
//! - mechanicsData 每条按 time 升序（SortByTime 稳定;同刻保玩家列表序）。
//!   玩家排序 Rust 为 Ordinal、C# culture —— 仅影响同刻 tie（日志罕见）。

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, AgentTable, NO_AGENT, species};

use crate::ctx::Ctx;
use crate::dto_a::{JsonMechanic, JsonMechanics};

/// 注册表条目（输出元数据；注册序 = 索引序）。
struct MechDef {
    id: i64,
    short: &'static str,
    full: &'static str,
    desc: &'static str,
    severity: &'static str,
}

fn registry() -> Vec<MechDef> {
    vec![
        // CommonMask 0x01000000 系 12 条（LogLogic.cs:99-132）
        MechDef { id: 0x0100_0001, short: "Dead", full: "Dead", desc: "Dead", severity: "Sev0" },
        MechDef { id: 0x0100_0002, short: "Downed", full: "Downed", desc: "Downed", severity: "Sev0" },
        MechDef { id: 0x0100_0003, short: "Got up", full: "Got up", desc: "Got up", severity: "Sev1" },
        MechDef { id: 0x0100_0004, short: "Res", full: "Res", desc: "Res", severity: "Sev2" },
        MechDef { id: 0x0100_0005, short: "DC", full: "DC", desc: "DC", severity: "Sev4" },
        MechDef { id: 0x0100_0006, short: "Resp", full: "Resp", desc: "Resp", severity: "Sev4" },
        MechDef {
            id: 0x0100_0007,
            short: "Knck.Dwn",
            full: "Knocked Down",
            desc: "Knocked Down",
            severity: "Sev4",
        },
        MechDef {
            id: 0x0100_0008,
            short: "Knck.Pll",
            full: "Knocked Back/Pulled",
            desc: "Knocked Back or Pulled",
            severity: "Sev4",
        },
        MechDef { id: 0x0100_0009, short: "Flt", full: "Float", desc: "Float", severity: "Sev4" },
        MechDef { id: 0x0100_000A, short: "Lnch", full: "Launched", desc: "Launched", severity: "Sev4" },
        MechDef {
            id: 0x0100_000B,
            short: "Lckt",
            full: "Lockout (Stun, Daze, Petrify, etc...)",
            desc: "Lockout",
            severity: "Sev4",
        },
        MechDef {
            id: 0x0100_000C,
            short: "Wtr.Flt.Snk",
            full: "Float or Sinked in Water",
            desc: "Float or Sinked",
            severity: "Sev4",
        },
        // WvW（WvWLogic.cs:46-56；0x07000001/2）
        MechDef {
            id: 0x0700_0001,
            short: "Kllng.Blw.Player",
            full: "Killing Blows to enemy Players",
            desc: "Killing Blows inflicted by Squad Players to enemy Players",
            severity: "Sev0",
        },
        MechDef {
            id: 0x0700_0002,
            short: "Kllng.Blw.Enemy",
            full: "Killing Blows received by enemies",
            desc: "Killing Blows inflicted enemy Players by Squad Players",
            severity: "Sev0",
        },
    ]
}

/// 一条 mechanic 事件（构建中间态）。
#[derive(Debug, Clone, Copy)]
struct MechEv {
    time: i64,
    /// actor agent（玩家 / 伪 target）。
    actor: AgentId,
}

/// 地址 → AgentId（rows.rs `resolve` 的同语义本地版）。
fn resolve(table: &mut AgentTable, addr: u64, time: i64) -> AgentId {
    if addr == 0 {
        return NO_AGENT;
    }
    let id = table.resolve_agent(addr, time);
    if id != NO_AGENT {
        return id;
    }
    table
        .all_ids()
        .into_iter()
        .find(|&x| table.slot(x).expect("slot").address == addr)
        .unwrap_or(NO_AGENT)
}

/// 构建 mechanics 列表（全无事件 → None；与 C# present 判空一致）。
pub fn build_mechanics(ctx: &Ctx) -> Option<Vec<JsonMechanics>> {
    if !ctx.opts.compute_mechanics {
        return None;
    }
    let log = ctx.log;
    let log_end = log.log_data.log_end;
    // ---- PlayerList（小队玩家;注册遍历序）----
    let players: Vec<AgentId> = log.players.iter().map(|p| p.agent).collect();
    let is_player = |id: AgentId| -> bool {
        id != NO_AGENT && players.iter().any(|&p| log.agents.englobing_root(p) == id)
    };
    let dummy = log.log_data.dummy_target_agent;
    let n = registry().len();
    let mut evs_by_mech: Vec<Vec<MechEv>> = vec![Vec::new(); n];

    // ---- 0..5：PlayerStatus 系 + Res（按玩家遍历）----
    for p in &players {
        // Dead(0)/Downed(1)/Got up(2)/DC(4)/Resp(5) 的 aux 桶
        for (slot, rows) in [
            (0usize, &ctx.aux.dead_rows),
            (1, &ctx.aux.down_rows),
            (2, &ctx.aux.up_rows),
            (4, &ctx.aux.despawn_rows),
            (5, &ctx.aux.spawn_rows),
        ] {
            if let Some(list) = rows.get(p) {
                for &e in list {
                    let t = log.events[e].time().unwrap_or(0);
                    evs_by_mech[slot].push(MechEv { time: t, actor: *p });
                }
            }
        }
        // Res(3)：cast 行 skill 1066（Resurrect;time = cast 起算点）
        if let Some(list) = ctx.aux.cast_by_caster.get(p) {
            for &e in list {
                if let CombatEvent::AnimatedCast(f) = &log.events[e]
                    && i64::from(f.skill_id) == 1066
                {
                    evs_by_mech[3].push(MechEv { time: f.time, actor: *p });
                }
            }
        }
    }

    // ---- 6..11：CC 六种（PlayerDstCrowdControlMechanic;has_cc 门控）----
    {
        let mut has_cc = false;
        let mut cc_all: Vec<(u32, MechEv)> = Vec::new();
        let mut resolver = log.agents.clone();
        for evt in &log.events {
            if let CombatEvent::CrowdControl(f) = evt {
                has_cc = true;
                cc_all.push((
                    f.skill.skill_id,
                    MechEv {
                        time: f.skill.time,
                        actor: resolve(&mut resolver, f.skill.to, f.skill.time),
                    },
                ));
            }
        }
        if has_cc {
            let cc_sets: [&[u32]; 6] = [
                &[23294],                       // KD
                &[23295],                       // KBP
                &[23296],                       // Float
                &[23297],                       // Launch
                &[23306, 23300, 23307],         // Lockout
                &[23298, 23304, 23305],         // FloatSinkWater
            ];
            for (slot, ids) in cc_sets.iter().enumerate() {
                for &(skill, ev) in &cc_all {
                    if ids.contains(&skill) && is_player(ev.actor) {
                        evs_by_mech[6 + slot].push(ev);
                    }
                }
            }
        }
    }

    // ---- 12：Kllng.Blw.Player（玩家含其 minion 的击杀行;To 为
    // NonSquadPlayer/WvW 物种）----
    for p in &players {
        let actor = crate::rows::ActorRows::new(log, &ctx.rows, *p);
        for hl in actor.health_out(0, log_end, None) {
            let Some(row) = crate::stats::row_view(ctx, &hl) else { continue };
            if !row.has_killed {
                continue;
            }
            let to_ok = log.agents.slot(row.to).is_some_and(|a| {
                a.agent_type == gw2ei_model::AgentType::NonSquadPlayer
                    || a.id == species::WORLD_VERSUS_WORLD
            });
            if to_ok {
                evs_by_mech[12].push(MechEv { time: row.time, actor: *p });
            }
        }
    }

    // ---- 13：Kllng.Blw.Enemy（伪 target 承伤击杀行;来源终主 Player 类型）----
    {
        let actor = crate::rows::ActorRows::new(log, &ctx.rows, dummy);
        for hl in actor.health_in(0, log_end, None) {
            let Some(row) = crate::stats::row_view(ctx, &hl) else { continue };
            if !row.has_killed {
                continue;
            }
            let credited = log.agents.final_master(row.from);
            let is_squad_player = log
                .agents
                .slot(credited)
                .is_some_and(|a| a.agent_type == gw2ei_model::AgentType::Player);
            if is_squad_player {
                evs_by_mech[13].push(MechEv { time: row.time, actor: dummy });
            }
        }
    }

    // ---- 输出：注册序、无事件剔除、每 mech 按 time 升序（稳定）----
    let mut results: Vec<JsonMechanics> = Vec::new();
    for (slot, def) in registry().iter().enumerate() {
        let mut evs = std::mem::take(&mut evs_by_mech[slot]);
        evs.sort_by_key(|e| e.time);
        evs.retain(|e| e.time <= log_end);
        if evs.is_empty() {
            continue;
        }
        let data: Vec<JsonMechanic> = evs
            .iter()
            .map(|e| {
                let a = log.agents.slot(e.actor).expect("mech actor slot");
                JsonMechanic {
                    time: e.time,
                    actor: a.character.clone(),
                    id: i64::from(a.id),
                    instid: i64::from(a.instid),
                    weight: 1.0,
                }
            })
            .collect();
        results.push(JsonMechanics {
            mechanics_data: data,
            id: def.id,
            name: def.short.to_string(),
            full_name: def.full.to_string(),
            description: def.desc.to_string(),
            internal_cooldown: None,
            is_achievement_eligibility: false,
            severity: def.severity.to_string(),
        });
    }
    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}
