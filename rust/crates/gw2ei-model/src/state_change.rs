//! `AddStateChangeEvent`（CombatEventFactory.cs:11-649）的 P1 子集。
//!
//! 按 `research/ei-combat-events.md` §1 的 F（状态）/G（metadata）组裁剪；
//! H 组（Effect/Marker/SquadMarker/Transformation/Missile/WvWObjective/
//! GadgetCapture/GadgetAnimation）与 C# 明确忽略的行记入
//! `UnsupportedEventKind` 计数，不产生事件。
//!
//! P1 门控与 C# 的差异（记录）：`PointOfView`/`Guild` 的 AnonymousPlayers
//! 过滤属于设置层（`EvtcParserSettings.AnonymousPlayers`，默认 false），
//! P1 未暴露设置 → 恒不过滤（与 C# 默认值行为一致）。

use gw2ei_parse::arc_builds;
use gw2ei_parse::{
    BreakbarState, BuffStackType, ContentLocal, EvtcCombatItem, Language, StateChange,
};

use crate::events::*;
use crate::factory::Collector;
use crate::spec::spec_from_prof_elite;

pub(crate) fn dispatch_state_change(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    use StateChange as S;
    match item.state_change {
        // ===== F 组：Combat 状态 =====
        S::EnterCombat | S::ExitCombat => {
            // Subgroup=DstAgent、Spec=GetSpec(Value, BuffDmg)
            //（CombatStatusEvent.cs:10-13）。
            let spec = spec_from_prof_elite(item.value as u32, item.buff_dmg as u32);
            let base = StatusEventFields {
                time: item.time,
                src: item.src_agent,
            };
            if item.state_change == S::EnterCombat {
                c.counts.enter_combat += 1;
                c.events.push(CombatEvent::EnterCombat(CombatStatusEventFields {
                    base,
                    subgroup: item.dst_agent,
                    spec,
                    base_spec: spec.base_spec(),
                }));
            } else {
                c.counts.exit_combat += 1;
                c.events.push(CombatEvent::ExitCombat(CombatStatusEventFields {
                    base,
                    subgroup: item.dst_agent,
                    spec,
                    base_spec: spec.base_spec(),
                }));
            }
        }
        S::ChangeUp => {
            c.counts.alive += 1;
            c.events.push(CombatEvent::Alive(status_fields(item)));
        }
        S::ChangeDead => {
            c.counts.dead += 1;
            c.events.push(CombatEvent::Dead(status_fields(item)));
        }
        S::ChangeDown => {
            c.counts.down += 1;
            c.events.push(CombatEvent::Down(status_fields(item)));
        }
        S::Spawn => {
            c.counts.spawn += 1;
            c.events.push(CombatEvent::Spawn(status_fields(item)));
        }
        S::Despawn => {
            c.counts.despawn += 1;
            c.events.push(CombatEvent::Despawn(status_fields(item)));
        }
        S::HealthUpdate => {
            let mut percent = round_hundredth(item.dst_agent as f64 / 100.0);
            // >100 压到 100（HealthUpdateEvent.cs:15-17）。
            if percent > 100.0 {
                percent = 100.0;
            }
            c.counts.health_update += 1;
            c.events.push(CombatEvent::HealthUpdate(HealthPercentEventFields {
                base: status_fields(item),
                percent,
            }));
        }
        S::BarrierUpdate => {
            let mut percent = round_hundredth(item.dst_agent as f64 / 100.0);
            // >100 置 0（BarrierUpdateEvent.cs:12-15，与 HealthUpdate 不同）。
            if percent > 100.0 {
                percent = 0.0;
            }
            c.counts.barrier_update += 1;
            c.events.push(CombatEvent::BarrierUpdate(HealthPercentEventFields {
                base: status_fields(item),
                percent,
            }));
        }
        S::MaxHealthUpdate => {
            c.counts.max_health_update += 1;
            c.events.push(CombatEvent::MaxHealthUpdate(MaxHealthEventFields {
                base: status_fields(item),
                max_health: item.dst_agent as i32,
            }));
        }
        S::TeamChange => {
            // TeamIDInto=DstAgent；TeamChangeOnDespawn=20240612 后才有
            // ComingFrom=Value（P1 世代恒有）。
            c.counts.team_change += 1;
            c.events.push(CombatEvent::TeamChange(TeamChangeEventFields {
                base: status_fields(item),
                team_id_into: item.dst_agent,
                team_id_coming_from: if c.build >= arc_builds::TEAM_CHANGE_ON_DESPAWN {
                    item.value as u64
                } else {
                    0
                },
            }));
        }
        S::Targetable => {
            // DstAgent>=2 表示 unsupported，忽略（CombatEventFactory.cs:126）。
            if item.dst_agent < 2 {
                let fields = TargetableEventFields {
                    base: status_fields(item),
                    targetable: item.dst_agent == 1,
                };
                // 连续同值去重（:129-140）。
                let src = fields.base.src;
                let last = c.last_targetable_by_src.get(&src).copied();
                if last != Some(fields.targetable) {
                    c.last_targetable_by_src.insert(src, fields.targetable);
                    c.counts.targetable += 1;
                    c.events.push(CombatEvent::Targetable(fields));
                }
            }
            // Visibility 门控世代：20260522 <= build < 20260527 的 Targetable
            // 行携带可见性（Value<2 才有效，:142-157）。
            if c.build >= arc_builds::VISIBILITY_IN_TARGETABLE_STATE_CHANGE
                && c.build < arc_builds::VISIBILITY_ON_STATE_CHANGE
                && item.value < 2
            {
                push_visibility(c, item, item.value == 1);
            }
        }
        S::StealthChange => {
            // DstAgent>=2 表示 unsupported，忽略（:573-589）。
            if item.dst_agent < 2 {
                push_visibility(c, item, item.dst_agent == 1);
            }
        }
        S::BreakbarState => {
            c.counts.breakbar_state += 1;
            c.events.push(CombatEvent::BreakbarState(BreakbarStateEventFields {
                base: status_fields(item),
                // C# `GetBreakbarState(Value)` 形参为 int（BreakbarStateEvent.cs:20）。
                state: BreakbarState::from_csharp_int(item.value),
            }));
        }
        S::BreakbarPercent => {
            // 100 × f32_bits(Value)，round 2，>100 压 100
            //（BreakbarPercentEvent.cs:10-17）。
            let raw = f32::from_bits(item.value as u32);
            let mut percent = round_hundredth(100.0 * f64::from(raw));
            if percent > 100.0 {
                percent = 100.0;
            }
            c.counts.breakbar_percent += 1;
            c.events.push(CombatEvent::BreakbarPercent(BreakbarPercentEventFields {
                base: status_fields(item),
                percent,
            }));
        }
        S::Glider => {
            c.counts.glider += 1;
            c.events.push(CombatEvent::Glider(GliderEventFields {
                base: status_fields(item),
                deployed: item.value == 1,
            }));
        }
        S::Last90BeforeDown => {
            c.counts.last90_before_down += 1;
            c.events.push(CombatEvent::Last90BeforeDown(Last90BeforeDownEventFields {
                base: status_fields(item),
                time_since_last_90: item.dst_agent,
            }));
        }
        S::Position | S::Rotation | S::Velocity | S::Teleport => {
            push_movement(c, item);
        }
        // ===== E 组：WeaponSwap（独立于动画 cast 的 state change 行）=====
        S::WeaponSwap => {
            // SwappedTo=(int)DstAgent、SwappedFrom=Value
            //（build ≥ 20240627 恒有；WeaponSwapEvent.cs:11-18）。
            c.counts.weapon_swap += 1;
            c.events.push(CombatEvent::WeaponSwap(WeaponSwapEventFields {
                time: item.time,
                caster: item.src_agent,
                swapped_to: item.dst_agent as i32,
                swapped_from: if c.build
                    >= arc_builds::WEAPON_SWAP_VALUE_IS_PREVIOUS_CROWD_CONTROL_EVENTS_GLIDER_EVENTS
                {
                    item.value
                } else {
                    -1
                },
            }));
        }
        // ===== C 组：StackActive/Deactive（新世代独立行）=====
        S::StackActive => {
            // To=SrcAgent、By=unknown（BuffStackEvent.cs:12-14）；
            // BuffInstance=(uint)DstAgent（BuffStackActiveEvent.cs:10-12）。
            c.counts.buff_stack_active += 1;
            c.events.push(CombatEvent::BuffStackActive(BuffStackEventFields {
                base: buff_stack_base_fields(item),
                buff_instance: item.dst_agent as u32,
            }));
        }
        S::StackDeactive => {
            // BuffInstance=Pad、ResetToDuration=Value
            //（BuffStackDeactiveEvent.cs:10-13）。
            c.counts.buff_stack_deactive += 1;
            c.events.push(CombatEvent::BuffStackDeactive(
                BuffStackDeactiveEventFields {
                    stack: BuffStackEventFields {
                        base: buff_stack_base_fields(item),
                        buff_instance: item.pad,
                    },
                    reset_to_duration: item.value,
                },
            ));
        }
        S::StunBreak => {
            // 方向修正 To=From、From=unknown（StunBreakEvent.cs:12-13）；
            // RemainingDuration=Value；AgainstDowned=offcycle==1（NonDamageEvent 基）。
            let skill = SkillEventFields {
                time: item.time,
                from: 0,
                to: item.src_agent,
                skill_id: item.skill_id,
                iff: item.iff,
                is_over_ninety: false,
                against_under_fifty: false,
                is_moving: false,
                against_moving: false,
                is_flanking: false,
                against_downed: item.is_offcycle == 1,
            };
            c.counts.stun_break += 1;
            c.events.push(CombatEvent::StunBreak(StunBreakEventFields {
                skill,
                remaining_duration: item.value,
            }));
        }
        // ===== G 组：metadata =====
        S::InstanceStart => {
            // TimeOffsetFromInstanceCreation = logStart - SrcAgent；InstanceIP
            // 由 Value 的 4 字节拼 IPv4（InstanceStartEvent.cs:12-20）。
            // P1 logStart=0（C# EvtcLogOffset；instance 日志 offset 恒 0）。
            let ip = if item.value != 0 {
                let v = item.value as u32;
                Some([
                    ((v >> 24) & 0xFF) as u8,
                    ((v >> 16) & 0xFF) as u8,
                    ((v >> 8) & 0xFF) as u8,
                    (v & 0xFF) as u8,
                ])
            } else {
                None
            };
            c.counts.metadata_singletons += 1;
            c.metadata.instance_start = Some(InstanceStartEventFields {
                time: item.time,
                // C# `logStart - (long)SrcAgent`(InstanceStartEvent.cs:13):
                // logStart 为归零前起点,不是 0 —— P1 以 wrapping 减法近似
                // 仅在日志起点为 0 时成立,此处改为显式原始起点。
                time_offset_from_instance_creation: c.log_start.wrapping_sub(item.src_agent as i64),
                instance_ip: ip,
            });
        }
        S::SquadCombatStart => {
            // Value==0 || BuffDmg==0 → 忽略（CombatEventFactory.cs:61-64）。
            if item.value == 0 || item.buff_dmg == 0 {
                c.counts.dropped_by_cs += 1;
                return;
            }
            let fields = SquadCombatStartEventFields {
                date: log_date_fields(item),
                log_type: item.squad_combat_log_type(),
            };
            c.counts.metadata_list += 1;
            c.metadata.squad_combat_starts.push(fields);
            // LogStartEvent：首个未被 LogEnd 顶替的（:66-71）。
            let expired_by_log_end = c
                .metadata
                .log_end_event
                .map(|end| end.date.server_unix_time_stamp <= fields.date.server_unix_time_stamp)
                .unwrap_or(false);
            if c.metadata.log_start_event.is_none() && !expired_by_log_end {
                c.metadata.log_start_event = Some(fields);
            }
        }
        S::SquadCombatEnd => {
            if item.value == 0 || item.buff_dmg == 0 {
                c.counts.dropped_by_cs += 1;
                return;
            }
            let fields = SquadCombatEndEventFields {
                date: log_date_fields(item),
                by_pov_map_exit: (item.dst_agent & 0x1) == 1,
            };
            c.counts.metadata_list += 1;
            c.metadata.log_end_event = Some(fields);
            c.metadata.squad_combat_ends.push(fields);
        }
        S::LogNPCUpdate => {
            c.counts.metadata_list += 1;
            c.metadata.log_npc_updates.push(LogNpcUpdateEventFields {
                date: log_date_fields(item),
                // AgentID=(ushort)SrcAgent（LogNPCUpdateEvent.cs:13）。
                agent_id: (item.src_agent & 0xFFFF) as u16,
                trigger_agent: item.dst_agent,
                trigger_is_gadget: item.is_flanking > 0,
            });
        }
        S::Reward => {
            c.counts.metadata_list += 1;
            c.metadata.rewards.push(RewardEventFields {
                time: item.time,
                reward_id: item.dst_agent,
                reward_type: item.value,
            });
        }
        S::Language => {
            c.counts.metadata_singletons += 1;
            c.metadata.language = Some(LanguageEventFields {
                language: Language::from_byte(item.src_agent as u8),
            });
        }
        S::GWBuild => {
            // SrcAgent==0 → 忽略（:100-106）。
            if item.src_agent == 0 {
                c.counts.dropped_by_cs += 1;
                return;
            }
            c.counts.metadata_singletons += 1;
            c.metadata.gw2_build = Some(Gw2BuildEventFields {
                build: item.src_agent,
            });
        }
        S::ShardID => {
            // Region：UserWorldID0>0 时本地判定；否则 C# 走 map API → P1
            // Unknown（ShardEvent.cs:25-41，P2 引入 map 数据后补齐）。
            let region = if item.value > 0 {
                Region::from_shard_id(item.value as u32 as u64)
            } else {
                Region::Unknown
            };
            c.counts.metadata_singletons += 1;
            c.metadata.shard = Some(ShardEventFields {
                shard_id: item.src_agent,
                upper_shard_id: item.dst_agent,
                user_world_id_0: item.value as u32,
                user_world_id_1: item.buff_dmg as u32,
                region,
            });
        }
        S::PointOfView => {
            // AnonymousPlayers 设置（默认 false）门控 —— P1 只判 src==0
            //（CombatEventFactory.cs:90-95）。
            if item.src_agent == 0 {
                c.counts.dropped_by_cs += 1;
                return;
            }
            c.counts.metadata_singletons += 1;
            c.metadata.point_of_view = Some(PointOfViewEventFields {
                time: item.time,
                pov: item.src_agent,
            });
        }
        S::MapID => {
            c.counts.metadata_singletons += 1;
            c.metadata.map_id = Some(MapIdEventFields {
                time: item.time,
                map_id: item.src_agent as i32,
                map_type: item.dst_agent,
            });
        }
        S::MapChange => {
            c.counts.metadata_list += 1;
            c.metadata.map_changes.push(MapChangeEventFields {
                time: item.time,
                map_id: item.src_agent as i32,
                old_map_id: item.dst_agent as i32,
                map_type: item.value,
            });
        }
        S::FractalScale => {
            // SrcAgent==0 → 忽略（:409-414）。
            if item.src_agent == 0 {
                c.counts.dropped_by_cs += 1;
                return;
            }
            c.counts.metadata_singletons += 1;
            c.metadata.fractal_scale = Some(FractalScaleEventFields {
                scale: item.src_agent as u8,
            });
        }
        S::WvWTeams => {
            c.counts.metadata_singletons += 1;
            c.metadata.wvw_teams = Some(wvw_teams_fields(item));
        }
        S::TickRate => {
            c.counts.metadata_list += 1;
            c.metadata.tick_rates.push(TickRateEventFields {
                time: item.time,
                tick_rate: item.src_agent,
            });
        }
        S::Tick => {
            c.counts.metadata_list += 1;
            c.metadata.ticks.push(TickEventFields {
                time: item.time,
                tick_interpolated: item.src_agent,
                tick_since_last_update: item.dst_agent,
            });
        }
        S::Integrity => {
            // ErrorEvent：前 32B 回写后按 UTF-8 解码（ErrorEvent.cs:11-23）。
            c.counts.metadata_list += 1;
            c.metadata.errors.push(ErrorEventFields {
                message: error_message(item),
            });
        }
        S::AttackTarget => {
            // 字段互换：Src 来自 DstAgent、AttackTarget 来自 SrcAgent
            //（AttackTargetEvent.cs:13-15）；Targetable=Value==1。
            c.counts.metadata_list += 1;
            c.metadata.attack_targets.push(AttackTargetEventFields {
                time: item.time,
                src: item.dst_agent,
                attack_target: item.src_agent,
                targetable: item.value == 1,
            });
        }
        S::Guild => {
            // AnonymousPlayers 门控同 PointOfView（默认 false → 恒构造）。
            c.counts.metadata_list += 1;
            c.metadata.guilds.push(GuildEventFields {
                src: item.src_agent,
                guild_key_hex: guild_guid(item),
            });
        }
        S::BuffInfo | S::BuffFormula => {
            // BuffInfo/BuffFormula 行按 SkillID 合并（:173-195）。
            let entry = c
                .metadata
                .buff_info_by_id
                .entry(item.skill_id)
                .or_insert_with(|| BuffInfoEventFields {
                    buff_id: item.skill_id,
                    info: None,
                    formulas: Vec::new(),
                });
            if item.state_change == S::BuffFormula {
                entry.formulas.push(buff_formula_row(item));
            } else {
                entry.info = Some(buff_info_row(item));
            }
            c.counts.metadata_list += 1;
        }
        S::SkillInfo | S::SkillTiming => {
            let entry = c
                .metadata
                .skill_info_by_id
                .entry(item.skill_id)
                .or_insert_with(|| SkillInfoEventFields {
                    skill_id: item.skill_id,
                    recharge: 0.0,
                    range0: 0.0,
                    range1: 0.0,
                    tooltip_time: 0.0,
                    timings: Vec::new(),
                });
            if item.state_change == S::SkillTiming {
                entry.timings.push(SkillTimingRowFields {
                    action_byte: item.src_agent as u8,
                    action: gw2ei_parse::SkillAction::from_byte(item.src_agent as u8),
                    at_millisecond: item.dst_agent,
                });
            } else {
                // SkillInfo 行：Time(8B)+SrcAgent(8B) 按 4 个 f32 拆
                //（SkillInfoEvent.cs:35-46）。
                let t = item.time.to_le_bytes();
                let s = item.src_agent.to_le_bytes();
                entry.recharge = f32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                entry.range0 = f32::from_le_bytes([t[4], t[5], t[6], t[7]]);
                entry.range1 = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                entry.tooltip_time = f32::from_le_bytes([s[4], s[5], s[6], s[7]]);
            }
            c.counts.metadata_list += 1;
        }
        S::IDToGUID => {
            // 按 ContentLocal（OverstackValue 低字节）分派
            //（CombatEventFactory.cs:363-407）。build ≥ 20260501 ≥
            // FunctionalIDToGUIDEvents(20220709) → 恒走新语义。
            let content = ContentLocal::from_byte(item.overstack_value as u8);
            let fields = GuidEventFields {
                guid_first8: item.src_agent,
                guid_last8: item.dst_agent,
                content_id: item.skill_id,
            };
            let evt = match content {
                ContentLocal::Effect => {
                    // P3b：effect GUID 表（C# EffectGUIDEventsByEffectID，
                    // EffectGUIDEvent.cs:12-23）。DefaultDuration 由 BuffDmg
                    // 的 f32 位型给出（arc ≥ EXTRA_DATA_IN_GUID_EVENTS），
                    // 否则恒 -1（Dummy 语义）；GUID hex 恒登记（finder 的
                    // GUID → EffectID 匹配用）。
                    let default_duration = if c.build >= arc_builds::EXTRA_DATA_IN_GUID_EVENTS {
                        f32::from_bits(item.buff_dmg as u32) as i64
                    } else {
                        -1
                    };
                    c.metadata.effect_guid_by_effect_id.insert(
                        i64::from(item.skill_id as i32),
                        EffectGuidInfo {
                            guid_hex: fields.guid_hex(),
                            default_duration,
                        },
                    );
                    CombatEvent::GuidEffect(fields)
                }
                ContentLocal::Marker => CombatEvent::GuidMarker(fields),
                ContentLocal::Skill => CombatEvent::GuidSkill(fields),
                ContentLocal::Species => CombatEvent::GuidSpecies(fields),
                ContentLocal::Team => CombatEvent::GuidTeam(fields),
                ContentLocal::Emote => CombatEvent::GuidEmote(fields),
                ContentLocal::Transformation => CombatEvent::GuidTransformation(fields),
                ContentLocal::Unknown => {
                    // C# default: break → 无输出。
                    c.counts.dropped_by_cs += 1;
                    return;
                }
            };
            c.counts.metadata_guid += 1;
            c.metadata.id_to_guid.push(evt);
        }
        // ===== H 组子集：Effect（P3b；CombatEventFactory.cs:331-362 与
        // 494-529 的 CBTS45/51 + Split 世代）。样本 arc 用 60-63 拆分形态
        // （EffectGroundCreate/EffectAgentCreate/Remove；实测样例 30k 行）；
        // 45/51 为旧世代 CBTS（保留实现）。
        S::Effect_45
        | S::Effect_51
        | S::EffectGroundCreate
        | S::EffectGroundRemove
        | S::EffectAgentCreate
        | S::EffectAgentRemove => dispatch_effect(c, item),
        S::Marker => c.bump_unsupported(UnsupportedEventKind::Marker),
        S::SquadMarker => c.bump_unsupported(UnsupportedEventKind::SquadMarker),
        S::Transformation => c.bump_unsupported(UnsupportedEventKind::Transformation),
        // MissileCreate 事件化(P3b finder 触发面;MissileEvent.cs);
        // Launch/Remove 的配对/命中消费在 P3c。
        S::MissileCreate => {
            c.events.push(CombatEvent::Missile(MissileEventFields {
                time: item.time,
                src: item.src_agent,
                skill_id: item.skill_id,
            }));
        }
        S::MissileLaunch | S::MissileRemove => {
            c.bump_unsupported(UnsupportedEventKind::Missile)
        }
        S::WvWObjectiveStatus => c.bump_unsupported(UnsupportedEventKind::WvWObjectiveStatus),
        S::GadgetAnimation => c.bump_unsupported(UnsupportedEventKind::GadgetAnimation),
        S::GadgetCaptureOutlineShow
        | S::GadgetCaptureSplitPercent
        | S::GadgetCaptureOutlineHide
        | S::GadgetCaptureOutlinePoint => {
            c.bump_unsupported(UnsupportedEventKind::GadgetCapture)
        }
        // C# "Ignore for now"（GadgetNameVisible/EffectMissileCreate）与无
        // case 的行（RuleSet 等 essential 先行处理后 default break）。
        S::GadgetNameVisible | S::EffectMissileCreate | S::RuleSet => {
            c.bump_unsupported(UnsupportedEventKind::IgnoredByCs)
        }
        // parse 层 IsValid 已排除的余项（Jump 等）与未知行 → 防御兜底。
        _ => c.counts.dropped_by_cs += 1,
    }
}

fn status_fields(item: &EvtcCombatItem) -> StatusEventFields {
    StatusEventFields {
        time: item.time,
        src: item.src_agent,
    }
}

fn buff_stack_base_fields(item: &EvtcCombatItem) -> BuffEventFields {
    BuffEventFields {
        time: item.time,
        buff_id: item.skill_id,
        // To=SrcAgent、By=unknown（BuffStackEvent.cs:12-14）。
        by: 0,
        to: item.src_agent,
        iff: item.iff,
    }
}

/// Visibility 事件（Targetable 门控世代 / StealthChange 行），连续同值去重
///（CombatEventFactory.cs:144-157, :573-589）。
fn push_visibility(c: &mut Collector<'_>, item: &EvtcCombatItem, visible: bool) {
    let fields = VisibilityEventFields {
        base: status_fields(item),
        visible,
    };
    let src = fields.base.src;
    let last = c.last_visibility_by_src.get(&src).copied();
    if last != Some(visible) {
        c.last_visibility_by_src.insert(src, visible);
        c.counts.visibility += 1;
        c.events.push(CombatEvent::Visibility(fields));
    }
}

/// Movement 坐标解包：DstAgent 低 8B = x/y 两个 f32、Value 位型 = z
///（MovementEvent.cs:31-38 的 UnpackMovementData）。
/// 非有限值（NaN/Inf）丢弃 —— C# 位于 replay 消费端（PositionEvent.cs 等
/// AddPoint3D），P1 提前到构造（模块文档记录）。
fn push_movement(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    let dst = item.dst_agent.to_le_bytes();
    let x = f32::from_le_bytes([dst[0], dst[1], dst[2], dst[3]]);
    let y = f32::from_le_bytes([dst[4], dst[5], dst[6], dst[7]]);
    let z = f32::from_bits(item.value as u32);
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        c.counts.dropped_by_cs += 1;
        return;
    }
    c.counts.movement += 1;
    let base = status_fields(item);
    let evt = match item.state_change {
        StateChange::Position => CombatEvent::Position(MovementEventFields { base, x, y, z }),
        StateChange::Rotation => CombatEvent::Rotation(MovementEventFields { base, x, y, z }),
        StateChange::Velocity => CombatEvent::Velocity(MovementEventFields { base, x, y, z }),
        StateChange::Teleport => CombatEvent::Teleport(MovementEventFields { base, x, y, z }),
        _ => unreachable!("push_movement only for movement state changes"),
    };
    c.events.push(evt);
}

/// LogDateEvent 字段：ServerUnixTimeStamp=(uint)Value、Local=(uint)BuffDmg
///（LogDateEvent.cs:10-15）。
fn log_date_fields(item: &EvtcCombatItem) -> LogDateEventFields {
    LogDateEventFields {
        time: item.time,
        server_unix_time_stamp: item.value as u32,
        local_unix_time_stamp: item.buff_dmg as u32,
    }
}

/// WvWTeams 24B 拆 6 个 u32（WvWTeamsEvent.cs:18-37 的 ByteBuffer 语义）：
/// 内存序 = src(8) dst(8) value(4) buffDmg(4)。
fn wvw_teams_fields(item: &EvtcCombatItem) -> WvWTeamsEventFields {
    let src = item.src_agent.to_le_bytes();
    let dst = item.dst_agent.to_le_bytes();
    let u32at = |b: &[u8], off: usize| {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    };
    WvWTeamsEventFields {
        red_shard_id: u32at(&src, 0),
        blue_shard_id: u32at(&src, 4),
        green_shard_id: u32at(&dst, 0),
        red_team_id: u32at(&dst, 4),
        blue_team_id: item.value as u32,
        green_team_id: item.buff_dmg as u32,
    }
}

/// Guild 事件 GUID：字节交换后拼 8-4-4-4-12 hex（GuildEvent.cs:12-53）。
fn guild_guid(item: &EvtcCombatItem) -> String {
    let d = item.dst_agent.to_le_bytes();
    let mut g = [0u8; 16];
    // ReverseEndianness(dst 低 32 位)：小端取数后大端落位。
    g[0..4].copy_from_slice(&[d[3], d[2], d[1], d[0]]);
    g[4] = d[5];
    g[5] = d[4];
    g[6] = d[7];
    g[7] = d[6];
    g[8..12].copy_from_slice(&item.value.to_le_bytes());
    g[12..16].copy_from_slice(&item.buff_dmg.to_le_bytes());
    let mut out = String::with_capacity(36);
    out.push_str(&hex_upper(&g[0..4]));
    out.push('-');
    out.push_str(&hex_upper(&g[4..6]));
    out.push('-');
    out.push_str(&hex_upper(&g[6..8]));
    out.push('-');
    out.push_str(&hex_upper(&g[8..10]));
    out.push('-');
    out.push_str(&hex_upper(&g[10..16]));
    out
}

/// 大写 hex（对齐 ParserHelper.AppendHexString，ParserHelper.cs:185-194）。
fn hex_upper(bytes: &[u8]) -> String {
    const CHARSET: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(CHARSET[((b & 0xF0) >> 4) as usize] as char);
        out.push(CHARSET[(b & 0x0F) as usize] as char);
    }
    out
}

/// ErrorEvent 消息：Time/Src/Dst/Value/BuffDmg 共 32B 回写后 UTF-8 解码
///（ErrorEvent.cs:11-23）。
fn error_message(item: &EvtcCombatItem) -> String {
    let mut buf = [0u8; 32];
    buf[0..8].copy_from_slice(&item.time.to_le_bytes());
    buf[8..16].copy_from_slice(&item.src_agent.to_le_bytes());
    buf[16..24].copy_from_slice(&item.dst_agent.to_le_bytes());
    buf[24..28].copy_from_slice(&item.value.to_le_bytes());
    buf[28..32].copy_from_slice(&item.buff_dmg.to_le_bytes());
    String::from_utf8_lossy(&buf).into_owned()
}

/// BuffInfo 行字段（BuffInfoEvent.cs:39-60）。C# 按 build ≥
/// BuffAttrFlatIncRemoved(20220308) 才读 Pad1/Pad2 —— P1 世代恒读。
fn buff_info_row(item: &EvtcCombatItem) -> BuffInfoRowFields {
    let pad = item.pad_bytes();
    let stacking_type_byte = pad.b1;
    BuffInfoRowFields {
        probably_invul: item.is_flanking > 0,
        probably_invert: item.is_shields > 0,
        category_byte: item.is_offcycle,
        stacking_type_byte,
        stacking_type: BuffStackType::from_byte(stacking_type_byte),
        probably_resistance: pad.b2 > 0,
        max_stacks: item.src_master_instid,
        duration_cap: item.overstack_value,
    }
}

/// BuffFormula 行：44B（Time..DstMasterInstid）回写后按 11 个 f32 拆
///（BuffFormula.cs:79-127）。Attr 的 `GetBuffAttribute` 求解留 P2。
fn buff_formula_row(item: &EvtcCombatItem) -> BuffFormulaRowFields {
    let mut buf = [0u8; 44];
    buf[0..8].copy_from_slice(&item.time.to_le_bytes());
    buf[8..16].copy_from_slice(&item.src_agent.to_le_bytes());
    buf[16..24].copy_from_slice(&item.dst_agent.to_le_bytes());
    buf[24..28].copy_from_slice(&item.value.to_le_bytes());
    buf[28..32].copy_from_slice(&item.buff_dmg.to_le_bytes());
    buf[32..36].copy_from_slice(&item.overstack_value.to_le_bytes());
    buf[36..38].copy_from_slice(&item.src_instid.to_le_bytes());
    buf[38..40].copy_from_slice(&item.dst_instid.to_le_bytes());
    buf[40..42].copy_from_slice(&item.src_master_instid.to_le_bytes());
    buf[42..44].copy_from_slice(&item.dst_master_instid.to_le_bytes());
    let floats: [f32; 11] = std::array::from_fn(|i| {
        let off = i * 4;
        f32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    });
    // C# (byte)float 显式转换：字节值域恒在 0-255（arc 把字节按 float 打包）。
    let byte_attr1 = floats[1] as u8;
    let byte_attr2 = floats[2] as u8;
    let pad = item.pad_bytes();
    BuffFormulaRowFields {
        // C# (int)formulaFloats[n] 是 float→int 数值转换（截断）；arc 把
        // 整数 id 按 float 打包，数值转换即还原整数。
        formula_type: floats[0] as i32,
        byte_attr1,
        byte_attr2,
        constant_offset: floats[3],
        level_offset: floats[4],
        variable: floats[5],
        trait_src: floats[6] as i32,
        trait_self: floats[7] as i32,
        content_reference: floats[8],
        buff_src: floats[9] as i32,
        buff_self: floats[10] as i32,
        extra_number: item.overstack_value,
        extra_number_state: pad.b1,
        npc: item.is_flanking == 0,
        player: item.is_shields == 0,
        is_break: item.is_offcycle > 0,
    }
}

/// `Math.Round(x, 2)`（banker's rounding，C# 默认 ToEven）。
fn round_hundredth(value: f64) -> f64 {
    (value * 100.0).round_ties_even() / 100.0
}

// ===== H 组子集：Effect CBTS45/51（P3b）=====

/// Effect_45/Effect_51 行事件化（CombatEventFactory.cs:331-362 +
/// EffectEvents/NonSplit 构造语义）。
///
/// - end 行（SkillID==0）：CBTS45 直接丢弃；CBTS51 构造
///   `EffectEndEventCBTS51`（按 TrackingID 把 end time 写进最近一个
///   start 事件，仅一次 —— SetDynamicEndTime）。
/// - start 行：CBTS51 的 Duration/TrackingID 由行偏移 48-59 的原始字节
///   重拼（EffectEventCBTS51.cs ReadDuration/ReadTrackingID），Duration
///   为 0 时按 GUID 事件 DefaultDuration 兜底；CBTS45 无 duration/
///   tracking（EffectEventCBTS45.cs）。
/// - `OnNonStaticPlatform`（IsFlanking>0）在 Release 构建直接丢弃
///   （`#if !DEBUG` 段），CLI 基准即 Release —— Rust 恒丢。
/// - 位置：DstAgent != 0 → `IsAroundDst`（dst 承载）；否则 Value/BuffDmg/
///   OverstackValue 三个 f32（NonSplitEffectEvent.cs:9-16）。
fn dispatch_effect(c: &mut Collector<'_>, item: &EvtcCombatItem) {
    use StateChange as S;
    // ---- Split 世代(60-63;样本实测形态;CombatEventFactory.cs:494-529)----
    // EffectGroundCreate/EffectAgentCreate:EffectID=SkillID、TrackingID=Pad、
    // Duration 由偏移 48-51 四字节(SplitEffectEvent.cs ReadDuration)、GUID
    // DefaultDuration 兜底;AgentCreate 的 DstAgent=承载 agent(IsAroundDst)。
    // GroundRemove/AgentRemove:end 行(TrackingID=Pad),按各自桶配对。
    match item.state_change {
        S::EffectGroundRemove => {
            let tracking = item.pad;
            if tracking != 0
                && let Some(list) = c.split_ground_by_tracking.get(&tracking)
                && let Some(&idx) = list.last()
                && let CombatEvent::Effect(f) = &mut c.events[idx]
                && f.end_time.is_none()
            {
                f.end_time = Some(item.time);
            }
            return;
        }
        S::EffectAgentRemove => {
            let tracking = item.pad;
            if tracking != 0
                && let Some(list) = c.split_agent_by_tracking.get(&tracking)
                && let Some(&idx) = list.last()
                && let CombatEvent::Effect(f) = &mut c.events[idx]
                && f.end_time.is_none()
            {
                f.end_time = Some(item.time);
            }
            return;
        }
        S::EffectGroundCreate | S::EffectAgentCreate => {
            let is_agent = item.state_change == S::EffectAgentCreate;
            let duration_raw =
                u32::from_le_bytes([item.iff_raw, item.buff, item.result, item.activation_raw]);
            let mut duration = i64::from(duration_raw);
            let eid = i64::from(item.skill_id as i32);
            if duration == 0
                && let Some(info) = c.metadata.effect_guid_by_effect_id.get(&eid)
                && info.default_duration > 0
            {
                duration = info.default_duration.min(i64::from(i32::MAX));
            }
            if item.is_flanking > 0 {
                // OnNonStaticPlatform:Release 丢弃(公共段 #if !DEBUG)
                return;
            }
            // Ground 位置:DstAgent(8B)+Value(4B) 六字节 → 3 short × 10;
            // Agent 位置无效(IsAroundDst)。
            let position = if is_agent {
                (0.0f32, 0.0f32, 0.0f32)
            } else {
                let mut b = [0u8; 12];
                b[..8].copy_from_slice(&item.dst_agent.to_le_bytes());
                b[8..12].copy_from_slice(&(item.value as u32).to_le_bytes());
                let s0 = i16::from_le_bytes([b[0], b[1]]);
                let s1 = i16::from_le_bytes([b[2], b[3]]);
                let s2 = i16::from_le_bytes([b[4], b[5]]);
                (
                    f32::from(s0) * 10.0,
                    f32::from(s1) * 10.0,
                    f32::from(s2) * 10.0,
                )
            };
            let idx = c.events.len();
            c.events.push(CombatEvent::Effect(EffectEventFields {
                time: item.time,
                src: item.src_agent,
                dst: if is_agent { item.dst_agent } else { 0 },
                effect_id: item.skill_id,
                duration,
                end_time: None,
                tracking_id: item.pad,
                cbts45: false,
                position,
            }));
            let tracking = item.pad;
            if tracking != 0 {
                if is_agent {
                    c.split_agent_by_tracking.entry(tracking).or_default().push(idx);
                } else {
                    c.split_ground_by_tracking.entry(tracking).or_default().push(idx);
                }
            }
            return;
        }
        _ => {}
    }
    let cbts45 = item.state_change == StateChange::Effect_45;
    if item.skill_id == 0 {
        // CBTS45 的 end 不支持直接 return;CBTS51 end(EffectEndEventCBTS51:
        // TrackingID 由 52-55 四字节拼)。
        if !cbts45 {
            let tracking =
                u32::from_le_bytes([item.buff_remove_raw, item.is_ninety, item.is_fifty, item.is_moving]);
            if tracking != 0
                && let Some(list) = c.effect_starts_by_tracking.get(&tracking)
                && let Some(&idx) = list.last()
                && let CombatEvent::Effect(f) = &mut c.events[idx]
                && f.end_time.is_none()
            {
                f.end_time = Some(item.time);
            }
        }
        return;
    }
    let duration_raw =
        u32::from_le_bytes([item.iff_raw, item.buff, item.result, item.activation_raw]);
    let tracking =
        u32::from_le_bytes([item.buff_remove_raw, item.is_ninety, item.is_fifty, item.is_moving]);
    let mut duration = i64::from(duration_raw);
    // GUID 表兜底(仅 CBTS51;Dummy GUID 的 DefaultDuration = -1 不覆盖)
    if !cbts45 && duration == 0 {
        let eid = i64::from(item.skill_id as i32);
        if let Some(info) = c.metadata.effect_guid_by_effect_id.get(&eid)
            && info.default_duration > 0
        {
            duration = info.default_duration.min(i64::from(i32::MAX));
        }
    }
    if item.is_flanking > 0 {
        // OnNonStaticPlatform:Release 丢弃(CombatEventFactory.cs:347-351)
        return;
    }
    let (dst, position) = if item.dst_agent != 0 {
        (item.dst_agent, (0.0f32, 0.0f32, 0.0f32))
    } else {
        (
            0,
            (
                f32::from_bits(item.value as u32),
                f32::from_bits(item.buff_dmg as u32),
                f32::from_bits(item.overstack_value),
            ),
        )
    };
    let idx = c.events.len();
    c.events.push(CombatEvent::Effect(EffectEventFields {
        time: item.time,
        src: item.src_agent,
        dst,
        effect_id: item.skill_id,
        duration,
        end_time: None,
        tracking_id: if cbts45 { 0 } else { tracking },
        cbts45,
        position,
    }));
    if !cbts45 && tracking != 0 {
        c.effect_starts_by_tracking
            .entry(tracking)
            .or_default()
            .push(idx);
    }
}
