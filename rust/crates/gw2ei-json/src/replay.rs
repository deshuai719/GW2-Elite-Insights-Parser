//! combatReplay 块（P4）：combatReplayMetaData + per-actor combatReplayData。
//!
//! C# 链路：WvWLogic.GetCombatMapInternal(:103-154) 建 `CombatReplayMap` →
//! SingleActor.InitCombatReplay（movement 事件 → PollingRate 采样 →
//! TrimCombatReplay）→ SingleActorCombatReplayDescription（positions/
//! orientations/dead/down/dc 拍平）。
//!
//! 采样与状态语义逐条对齐 C#（CombatReplay.cs:200-243 / SingleActor.cs:324-418 /
//! SingleActorStatusHelper.cs:76-177）;坐标为 WvW 大陆坐标 → GetMapCoordRounded
//! 像素化（CombatReplayMap.cs:178-194）。角度 = `-GetRoundedZRotationDeg`
//!（atan2(y,x) 度、3 位银行家舍入、f32）。

use gw2ei_model::{AgentId, AgentType};

use crate::ctx::Ctx;
use crate::dto_a::{CombatReplayMapMeta, CombatReplayMetaData};
use crate::dto_b::JsonActorCombatReplayData;
use crate::rows::MoveKind;

// ===== 地图常量（WvWLogic.GetCombatMapInternal）=====

pub struct CrMap {
    /// 图片像素 (w, h)。
    pixel: (f64, f64),
    /// rectInMap (topX, topY, bottomX, bottomY)。
    rect: (f64, f64, f64, f64),
}

/// 地图 url（LogImages.cs CombatReplay 常量;WvW 地图子集;
/// trellis-check P4 审计核对 3 处与 C# 不符的 URL 已修正）。
fn map_url(map_id: i32) -> &'static str {
    match map_id {
        38 => "https://i.imgur.com/t0khtQd.png",   // Eternal Battlegrounds
        95 | 96 => "https://i.imgur.com/nVu2ivF.png", // Alpine Borderlands
        1099 => "https://i.imgur.com/R5p9fqw.png",  // Red Desert Borderlands
        968 => "https://i.imgur.com/iEpKYL0.jpg",   // Edge of the Mists
        _ => "",
    }
}

/// GetCombatMapInternal 的 WvW 面（非 WvW 地图 → 800×800 退化 rect，与 C#
/// LogLogic.GetCombatMapInternal 默认一致 —— 本工程只跑 WvW）。
fn cr_map_for(map_id: i32) -> (CrMap, &'static str) {
    let m = match map_id {
        38 => CrMap {
            pixel: (954.0, 1000.0),
            rect: (-36864.0 + 950.0, -36864.0 + 2250.0, 36864.0 + 950.0, 36864.0 + 2250.0),
        },
        95 | 96 => CrMap { pixel: (697.0, 1000.0), rect: (-30720.0, -43008.0, 30720.0, 43008.0) },
        1099 => CrMap { pixel: (1000.0, 1000.0), rect: (-36864.0, -36864.0, 36864.0, 36864.0) },
        968 => CrMap { pixel: (3556.0, 3646.0), rect: (-36864.0, -36864.0, 36864.0, 36864.0) },
        _ => CrMap { pixel: (800.0, 800.0), rect: (0.0, 0.0, 0.0, 0.0) },
    };
    (m, map_url(map_id))
}

impl CrMap {
    /// `GetPixelMapSize`（CombatReplayMap.cs:88-104）：最长边缩到 750。
    fn pixel_map_size(&self) -> (i64, i64) {
        let ratio = self.pixel.0 / self.pixel.1;
        if ratio > 1.0 {
            (750, round_ties(750.0 / ratio) as i64)
        } else if ratio < 1.0 {
            (round_ties(ratio * 750.0) as i64, 750)
        } else {
            (750, 750)
        }
    }

    /// `GetInchToPixel`（:196-200）：ratio = rectWidth/pixelWidth(f32 转换)，
    /// round(1/ratio, 3)。
    fn inch_to_pixel(&self) -> f64 {
        let w = self.pixel_map_size().0 as f32;
        let rect_w = (self.rect.2 - self.rect.0) as f32;
        let ratio = rect_w / w;
        round_3(f64::from(1.0f32 / ratio))
    }

    /// `GetMapCoordRounded`（:178-194）：3 位圆整 + y 翻转。
    fn map_coord_rounded(&self, real_x: f32, real_y: f32) -> (f32, f32) {
        let (w, h) = self.pixel_map_size();
        let (pw, ph) = self.pixel;
        let scale_x = w as f64 / pw;
        let scale_y = h as f64 / ph;
        let x = (f64::from(real_x) - self.rect.0) / (self.rect.2 - self.rect.0);
        let y = (f64::from(real_y) - self.rect.1) / (self.rect.3 - self.rect.1);
        (
            round_3(scale_x * pw * x) as f32,
            round_3(scale_y * (ph - ph * y)) as f32,
        )
    }
}

/// Math.Round(x, 3)（banker's rounding;digits=CombatReplayDataDigit）。
fn round_3(v: f64) -> f64 {
    (v * 1000.0).round_ties_even() / 1000.0
}

/// Math.Round(x)（整数;banker's;用于 GetPixelMapSize）。
fn round_ties(v: f64) -> f64 {
    v.round_ties_even()
}

// ===== per-actor 段 =====

pub struct ActorReplay {
    pub start: i64,
    pub end: i64,
    /// 像素坐标对（Trim 后）。
    pub positions: Vec<(f32, f32)>,
    pub orientations: Vec<f32>,
    pub dead: Vec<(i64, i64)>,
    pub down: Vec<(i64, i64)>,
    pub dc: Vec<(i64, i64)>,
}

/// 采样点（ParametricPoint3D 等价）。
#[derive(Clone, Copy)]
struct Pt {
    time: i64,
    x: f32,
    y: f32,
    z: f32,
}

/// 构建单 actor 的 replay 数据（movement + 状态 + Trim）。
/// `force_polling`：C# PollingRate 的 `AgentItem.Type == Player` 参数。
pub fn actor_replay(ctx: &Ctx, agent: AgentId, map: &CrMap) -> ActorReplay {
    let log = ctx.log;
    let log_duration = log.log_data.log_end;
    let a = log.agents.slot(agent).expect("replay actor slot");
    let first_aware = a.first_aware;
    let last_aware = a.last_aware;
    let force_polling = a.agent_type == AgentType::Player;

    // ---- movement 原始流（C# InitCombatReplay:逐事件 AddPoint3D）----
    let mut positions: Vec<Pt> = Vec::new();
    let mut velocities: Vec<Pt> = Vec::new();
    let mut rotations: Vec<Pt> = Vec::new();
    if let Some(rows) = ctx.aux.movement_rows.get(&agent) {
        for l in rows {
            // AddPoint3D 消费端过滤（PositionEvent.cs:11-20 / TeleportEvent.cs /
            // RotationEvent.cs / VelocityEvent.cs;NaN/Inf 已在模型层丢弃）。
            match l.kind {
                MoveKind::Position | MoveKind::Teleport => {
                    if l.x == 0.0 && l.y == 0.0 && l.z == 0.0 {
                        continue;
                    }
                    if l.x * l.x + l.y * l.y > 16e8 {
                        continue;
                    }
                    if l.kind == MoveKind::Position {
                        positions.push(Pt { time: l.time, x: l.x, y: l.y, z: l.z });
                    } else {
                        // AddTeleport:停插补的零速点 + 恢复点（CombatReplay.cs:47-58）
                        if let Some(&last) = velocities.last() {
                            let t0 = (last.time + 1).min(l.time - 1);
                            velocities.push(Pt { time: t0, x: 0.0, y: 0.0, z: 0.0 });
                            velocities.push(Pt { time: l.time, x: last.x, y: last.y, z: last.z });
                        }
                        positions.push(Pt { time: l.time, x: l.x, y: l.y, z: l.z });
                    }
                }
                MoveKind::Velocity => {
                    velocities.push(Pt { time: l.time, x: l.x, y: l.y, z: l.z });
                }
                MoveKind::Rotation => {
                    rotations.push(Pt { time: l.time, x: l.x, y: l.y, z: l.z });
                }
            }
        }
    }

    // ---- PollingRate（CombatReplay.cs:200-243）----
    // C#：positions 空时 forcePolling(Player) 塞占位点后继续;NPC 直接返回。
    if positions.is_empty() && !force_polling {
        positions.clear();
        rotations.clear();
    } else {
        if positions.is_empty() {
            positions.push(Pt {
                time: 0,
                x: -2_147_483_648.0f32,
                y: -2_147_483_648.0f32,
                z: 0.0,
            });
        }
        let rate = 300i64;
        let do_rot = !rotations.is_empty();
        let pos_start = (rate * (positions[0].time / rate - 1)).min(0);
        let rot_start = if do_rot {
            (rate * (rotations[0].time / rate - 1)).min(0)
        } else {
            0
        };
        let start_offset = pos_start.min(rot_start);
        let capacity = ((log_duration - start_offset) / rate + 1) as usize;
        let mut polled_pos: Vec<Pt> = Vec::with_capacity(capacity);
        let mut polled_rot: Vec<Pt> = Vec::with_capacity(if do_rot { capacity } else { 0 });
        let mut t = start_offset;
        let mut p_idx = 0usize;
        let mut v_idx = -1i64;
        let mut r_idx = 0usize;
        while t < log_duration {
            handle_position(t, &positions, &velocities, &mut polled_pos, &mut p_idx, &mut v_idx);
            if do_rot {
                handle_rotation(t, &rotations, &mut polled_rot, &mut r_idx);
            }
            t += rate;
        }
        debug_assert!(polled_pos.len() <= capacity && polled_rot.len() <= capacity);
        // C# 定长数组的尾部默认值(理论不可达;防呆截断)
        polled_pos.truncate(capacity);
        polled_rot.truncate(capacity);

        // ---- Trim（SingleActor.cs:347-359 的 replay.Trim）----
        positions = polled_pos;
        rotations = polled_rot;
    }

    // ---- 状态（SingleActorStatusHelper FillStatus/GetAgentStatus）----
    // C# 事件链接把「死后仍指向该地址」的行路由到 unknown/新 piece —— Rust
    // P2a 链接按地址延伸 aware;末状态为 Dead/Despawn 的 piece 以该时刻为
    // 有效终点复刻(否则 dc/dead 段与 Trim 会越过死亡时刻)。
    let (deads, downs, dcs, actives, eff_last_aware) =
        fill_status(ctx, agent, first_aware, last_aware);
    let last_aware = eff_last_aware;

    // ---- TrimCombatReplay（SingleActor.cs:324-381）----
    // 段集合按 C# 演员类分：PlayerActor（Player + NonSquadPlayer，
    // PlayerActor.cs:57-64）= deads+downs+actives 排序合并;其余（NPC/
    // minion 默认 SingleActor）= actives 仅 —— trellis-check P4 审计：
    // 此前无条件用玩家集合,NPC piece 在 downed/dead 尾部会把窗口延伸过
    // 最后 active 段（如龟 minion end 66455 vs 官方 41970）。
    let segments: Vec<Seg> = if a.agent_type.is_player() {
        let mut s = deads.to_vec();
        s.extend(downs.iter().copied());
        s.extend(actives.iter().copied());
        s.sort_by_key(|x| x.start);
        s
    } else {
        actives.clone()
    };
    let (trim_start, trim_end) = match (segments.first(), segments.last()) {
        (Some(f), Some(l)) => (f.start, l.end),
        _ => (first_aware, last_aware),
    };
    let trim_start = trim_start.max(first_aware);
    let trim_end = trim_end.min(last_aware);
    // replay.Trim(max(trimStart, FA), min(trimEnd, LA))：_start/=logStart(0)
    // 起夹;点位按 [start, end] 保留。
    let start = trim_start.max(0);
    let end = trim_end.max(start).min(log_duration);
    positions.retain(|p| p.time >= start && p.time <= end);
    rotations.retain(|p| p.time >= start && p.time <= end);

    // ---- 坐标转换 + 角度（SingleActorCombatReplayDescription.cs:56-72）----
    let positions_out: Vec<(f32, f32)> = positions
        .iter()
        .map(|p| map.map_coord_rounded(p.x, p.y))
        .collect();
    let orientations: Vec<f32> = rotations
        .iter()
        .map(|r| -round_3_deg_radians(r.x, r.y))
        .collect();

    ActorReplay {
        start,
        end,
        positions: positions_out,
        orientations,
        dead: deads.into_iter().map(|s| (s.start, s.end)).collect(),
        down: downs.into_iter().map(|s| (s.start, s.end)).collect(),
        dc: dcs.into_iter().map(|s| (s.start, s.end)).collect(),
    }
}

/// `GetRoundedZRotationDeg`：atan2(y,x) f32 → 度(double) → round 3 → f32。
fn round_3_deg_radians(x: f32, y: f32) -> f32 {
    let rad = f64::from(y.atan2(x));
    let deg = rad * 180.0 / std::f64::consts::PI;
    round_3(deg) as f32
}

/// HandlePosition（CombatReplay.cs:110-157）。
#[allow(clippy::too_many_arguments)]
fn handle_position(
    t: i64,
    positions: &[Pt],
    velocities: &[Pt],
    out: &mut Vec<Pt>,
    p_idx: &mut usize,
    v_idx: &mut i64,
) {
    let pos = positions[*p_idx];
    if t <= pos.time {
        out.push(Pt { time: t, x: pos.x, y: pos.y, z: pos.z });
        return;
    }
    if *p_idx == positions.len() - 1 {
        out.push(Pt { time: t, x: pos.x, y: pos.y, z: pos.z });
        return;
    }
    let next = positions[*p_idx + 1];
    if next.time < t {
        *p_idx += 1;
        handle_position(t, positions, velocities, out, p_idx, v_idx);
        return;
    }
    let last = out
        .last()
        .filter(|p| p.time > pos.time)
        .copied()
        .unwrap_or(pos);
    *v_idx = update_velocity_index(velocities, t, *v_idx);
    let velocity = if *v_idx >= 0 && (*v_idx as usize) < velocities.len() {
        velocities[*v_idx as usize]
    } else {
        Pt { time: 0, x: 0.0, y: 0.0, z: 0.0 }
    };
    if velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z < 1e-6 {
        out.push(Pt { time: t, x: pos.x, y: pos.y, z: pos.z });
    } else {
        // Vector3.Lerp(last, next, ratio)（f32;ratio 夹 [0,1]）
        let ratio = ((t - last.time) as f32 / (next.time - last.time) as f32).clamp(0.0, 1.0);
        out.push(Pt {
            time: t,
            x: last.x + (next.x - last.x) * ratio,
            y: last.y + (next.y - last.y) * ratio,
            z: last.z + (next.z - last.z) * ratio,
        });
    }
}

/// UpdateVelocityIndex（CombatReplay.cs:90-108，含 currentIndex 语义）。
fn update_velocity_index(velocities: &[Pt], time: i64, current: i64) -> i64 {
    if velocities.is_empty() {
        return -1;
    }
    let mut res = current.max(0) as usize;
    while res < velocities.len() && velocities[res].time < time {
        res += 1;
    }
    res as i64 - 1
}

/// HandleRotation（CombatReplay.cs:159-194;跨 >310ms 间隙不插补）。
fn handle_rotation(t: i64, rotations: &[Pt], out: &mut Vec<Pt>, r_idx: &mut usize) {
    let rot = rotations[*r_idx];
    if t <= rot.time {
        out.push(Pt { time: t, x: rot.x, y: rot.y, z: rot.z });
        return;
    }
    if *r_idx == rotations.len() - 1 {
        out.push(Pt { time: t, x: rot.x, y: rot.y, z: rot.z });
        return;
    }
    let next = rotations[*r_idx + 1];
    if next.time < t {
        *r_idx += 1;
        handle_rotation(t, rotations, out, r_idx);
        return;
    }
    let last = out
        .last()
        .filter(|p| p.time > rot.time)
        .copied()
        .unwrap_or(rot);
    if next.time - last.time > 310 {
        // ArcDPSPollingRate(300) + ServerDelayConstant(10)
        out.push(Pt { time: t, x: rot.x, y: rot.y, z: rot.z });
    } else {
        let ratio = ((t - last.time) as f32 / (next.time - last.time) as f32).clamp(0.0, 1.0);
        out.push(Pt {
            time: t,
            x: last.x + (next.x - last.x) * ratio,
            y: last.y + (next.y - last.y) * ratio,
            z: last.z + (next.z - last.z) * ratio,
        });
    }
}

/// 状态段（Segment）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seg {
    start: i64,
    end: i64,
}

/// GetStatus 状态段（SingleActorStatusHelper.cs:76-177 的常规面;englobed/
/// regrouped 合成 WvW 样例无分裂,注释锚点不实现）。返回第五元组 = 有效
/// last aware（末状态 Dead/Despawn → 该事件时刻）。
#[allow(clippy::type_complexity)]
fn fill_status(
    ctx: &Ctx,
    agent: AgentId,
    first_aware: i64,
    last_aware: i64,
) -> (Vec<Seg>, Vec<Seg>, Vec<Seg>, Vec<Seg>, i64) {
    let log = ctx.log;
    // C# 合并序:down → alive → dead → spawn → despawn(组内时间序)
    // kind:0=down,1=alive,2=dead,3=spawn,4=despawn
    let mut status: Vec<(i64, u8)> = Vec::new();
    let mut push_kind = |rows: &std::collections::BTreeMap<AgentId, Vec<usize>>, kind: u8| {
        if let Some(list) = rows.get(&agent) {
            for &e in list {
                if let Some(t) = log.events[e].time() {
                    status.push((t, kind));
                }
            }
        }
    };
    push_kind(&ctx.aux.down_rows, 0);
    push_kind(&ctx.aux.up_rows, 1);
    push_kind(&ctx.aux.dead_rows, 2);
    push_kind(&ctx.aux.spawn_rows, 3);
    push_kind(&ctx.aux.despawn_rows, 4);
    // C# OrderBy(Time) 稳定(同刻保 down→alive→dead→spawn→despawn 序)
    status.sort_by_key(|&(t, _)| t);
    // 有效终点:末状态 Dead/Despawn 时以该事件时刻收口(见 actor_replay 注)
    let last_aware = match status.last() {
        Some(&(t, k)) if k == 2 || k == 4 => t,
        _ => last_aware,
    };

    let (mut deads, mut downs, mut dcs, mut actives): (Vec<Seg>, Vec<Seg>, Vec<Seg>, Vec<Seg>) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let push_seg = |v: &mut Vec<Seg>, s: i64, e: i64| {
        if s < e {
            v.push(Seg { start: s, end: e });
        }
    };
    // AddSegment(dc, long.MinValue, FirstAware)
    push_seg(&mut dcs, i64::MIN, first_aware);
    if status.is_empty() {
        // actives=[FA, LA];dc=[LA, MaxValue]
        push_seg(&mut actives, first_aware, last_aware);
        push_seg(&mut dcs, last_aware, i64::MAX);
        return (deads, downs, dcs, actives, last_aware);
    }
    for i in 0..status.len() - 1 {
        add_value_to_status_list(
            &mut deads, &mut downs, &mut dcs, &mut actives, &push_seg,
            status[i], status[i + 1].0, first_aware, i == 0,
        );
    }
    let last = status[status.len() - 1];
    add_value_to_status_list(
        &mut deads, &mut downs, &mut dcs, &mut actives, &push_seg,
        last, last_aware, first_aware, status.len() - 1 == 0,
    );
    if last.1 == 2 {
        // 末状态 Dead → dead 延到 +inf;否则 dc 到 +inf
        push_seg(&mut deads, last_aware, i64::MAX);
    } else {
        push_seg(&mut dcs, last_aware, i64::MAX);
    }
    (deads, downs, dcs, actives, last_aware)
}

/// AddValueToStatusList(SingleActorStatusHelper.cs:40-74)。
#[allow(clippy::too_many_arguments)]
fn add_value_to_status_list(
    deads: &mut Vec<Seg>,
    downs: &mut Vec<Seg>,
    dcs: &mut Vec<Seg>,
    actives: &mut Vec<Seg>,
    push_seg: &impl Fn(&mut Vec<Seg>, i64, i64),
    cur: (i64, u8),
    next_time: i64,
    first_aware: i64,
    first: bool,
) {
    let c_time = cur.0;
    match cur.1 {
        0 => {
            // DownEvent
            if first {
                push_seg(actives, first_aware, c_time);
            }
            push_seg(downs, c_time, next_time);
        }
        2 => {
            // DeadEvent
            if first {
                push_seg(actives, first_aware, c_time);
            }
            push_seg(deads, c_time, next_time);
        }
        4 => {
            // DespawnEvent
            if first {
                push_seg(actives, first_aware, c_time);
            }
            push_seg(dcs, c_time, next_time);
        }
        _ => {
            // Alive/Spawn(else 分支)
            if first && c_time - first_aware > 50 {
                push_seg(dcs, first_aware, c_time);
            }
            push_seg(actives, c_time, next_time);
        }
    }
}

// ===== JSON 组装 =====

/// CanCombatReplay（ParsedEvtcLog.cs：settings.ComputeCombatReplay &&
/// CombatData.HasMovementData=movement 行 > 1）。
pub fn can_combat_replay(ctx: &Ctx) -> bool {
    ctx.opts.compute_combat_replay && movement_count(ctx) > 1
}

/// movement 事件总数（C# `_statusEvents.MovementEvents` 的列表计数）。
fn movement_count(ctx: &Ctx) -> usize {
    ctx.aux.movement_rows.values().map(Vec::len).sum()
}

/// JsonCombatReplayMetaData（JsonCombatReplayMetaDataBuilder.cs）——
/// WvW 的 maps = 单 ArenaDecoration 常量面（url/[logStart, logEnd]/[0,0]）。
pub fn meta_data(ctx: &Ctx) -> Option<CombatReplayMetaData> {
    if !can_combat_replay(ctx) {
        return None;
    }
    let (map, url) = cr_map_for(ctx.map_id as i32);
    let (w, h) = map.pixel_map_size();
    Some(CombatReplayMetaData {
        inch_to_pixel: map.inch_to_pixel(),
        polling_rate: 300,
        sizes: vec![w, h],
        maps: vec![CombatReplayMapMeta {
            url: url.to_string(),
            interval: vec![ctx.log_start, ctx.log_end],
            position: vec![0, 0],
        }],
    })
}

/// per-actor JsonActorCombatReplayData（JsonActorCombatReplayDataBuilder.cs）。
pub fn actor_cr_data(ctx: &Ctx, agent: AgentId, icon_url: &str) -> JsonActorCombatReplayData {
    let (map, _) = cr_map_for(ctx.map_id as i32);
    let r = actor_replay(ctx, agent, &map);
    let pairs = |v: &[(i64, i64)]| v.iter().map(|&(s, e)| vec![s, e]).collect();
    JsonActorCombatReplayData {
        start: r.start,
        end: r.end,
        icon_url: icon_url.to_string(),
        positions: r.positions.iter().map(|&(x, y)| vec![x, y]).collect(),
        orientations: r.orientations,
        dead: pairs(&r.dead),
        down: pairs(&r.down),
        dc: pairs(&r.dc),
    }
}

/// actor icon（PlayerActor.GetIcon(true)/NPC.GetIcon）—— JSON CR iconURL。
pub fn actor_icon(ctx: &Ctx, agent: AgentId) -> String {
    let icons = &ctx.content.icons;
    let Some(a) = ctx.log.agents.slot(agent) else {
        return icons.fallback_npc_generic.clone();
    };
    if a.agent_type.is_player() {
        icons.prof_of(a.spec.csharp_name()).to_string()
    } else if a.agent_type == AgentType::VolatileSpecies {
        icons.fallback_gadget.clone()
    } else {
        icons.npc_of(a.id).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpine() -> CrMap {
        cr_map_for(96).0
    }

    #[test]
    fn pixel_map_size_and_inch_alpine() {
        // golden 实测 [523, 750] + 0.009
        let m = alpine();
        assert_eq!(m.pixel_map_size(), (523, 750));
        assert_eq!(m.inch_to_pixel(), 0.009);
    }

    #[test]
    fn map_coord_rounded_mid_and_flip() {
        // 地图中心 (0,0) → (523/2=261.5, 750/2=375.0); y 翻转轴
        let m = alpine();
        let (x, y) = m.map_coord_rounded(0.0, 0.0);
        assert_eq!((x as f64, y as f64), (261.5, 375.0));
        // 顶角 (-30720, 43008) → (0, 0)
        let (x, y) = m.map_coord_rounded(-30720.0, 43008.0);
        assert_eq!((x as f64, y as f64), (0.0, 0.0));
        // 底角 (30720, -43008) → (523, 750)
        let (x, y) = m.map_coord_rounded(30720.0, -43008.0);
        assert_eq!((x as f64, y as f64), (523.0, 750.0));
    }

    #[test]
    fn angle_deg_negated() {
        assert_eq!(round_3_deg_radians(1.0, 0.0), 0.0);
        assert_eq!(round_3_deg_radians(0.0, 1.0), 90.0);
        assert_eq!(round_3_deg_radians(-1.0, 0.0), 180.0);
    }

    #[test]
    fn teleport_velocity_inserts() {
        // AddTeleport:已有 velocity 时插零速 + 恢复点
        let mut positions: Vec<Pt> = Vec::new();
        let mut velocities: Vec<Pt> = vec![Pt { time: 100, x: 1.0, y: 0.0, z: 0.0 }];
        // 复用 AddPoint3D 过滤的 Teleport 路径(手动模拟)
        let p = Pt { time: 500, x: 10.0, y: 0.0, z: 0.0 };
        if let Some(&last) = velocities.last() {
            let t0 = (last.time + 1).min(p.time - 1);
            velocities.push(Pt { time: t0, x: 0.0, y: 0.0, z: 0.0 });
            velocities.push(Pt { time: p.time, x: last.x, y: last.y, z: last.z });
        }
        positions.push(p);
        assert_eq!(velocities.len(), 3);
        assert_eq!(velocities[1].time, 101);
        assert_eq!(velocities[2].time, 500);
    }
}
