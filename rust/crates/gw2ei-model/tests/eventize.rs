//! 事件化分发测试：合成 CombatItem 流 → 期望的语义事件与字段。
//!
//! 语义对齐 C#（`CombatData.cs` 构造器 + `CombatEventFactory.cs` + 各事件
//! 类构造器），字段断言逐项锁定。CombatItem 直接构造 struct（与 parse 层
//! 输出同构），绕过二进制层以便聚焦分发逻辑。

use gw2ei_model::{CombatData, ModelError};
use gw2ei_parse::skill_ids;
use gw2ei_parse::{EvtcCombatItem, EvtcRawAgent, EvtcRawLog, EvtcRawSkill, Iff, StateChange};

// ===== 合成工具 =====

/// Combat 行（默认：src 非 0、IFF Friend、result 0=DirectNormal）。
fn combat_row() -> EvtcCombatItem {
    EvtcCombatItem {
        time: 5000,
        src_agent: 0x1111,
        dst_agent: 0x2222,
        value: 0,
        buff_dmg: 0,
        overstack_value: 0,
        skill_id: 7,
        src_instid: 1,
        dst_instid: 2,
        src_master_instid: 0,
        dst_master_instid: 0,
        iff: Iff::Friend,
        iff_raw: 0,
        buff: 0,
        result: 0,
        activation: gw2ei_parse::Activation::None,
        activation_raw: 0,
        buff_remove: gw2ei_parse::BuffRemove::None,
        buff_remove_raw: 0,
        is_ninety: 0,
        is_fifty: 0,
        is_moving: 0,
        state_change: StateChange::Combat,
        state_change_raw: 0,
        is_flanking: 0,
        is_shields: 0,
        is_offcycle: 0,
        pad: 0,
    }
}

/// state change 行（默认 Combat 外 statechange）。
fn state_row(state: StateChange) -> EvtcCombatItem {
    let mut r = combat_row();
    r.state_change = state;
    r.state_change_raw = state as u8;
    r
}

/// 玩家 agent（地址 0x1111）。
fn player_agent(address: u64) -> EvtcRawAgent {
    let mut name = [0u8; 68];
    name[..9].copy_from_slice(b"PlayerOne");
    EvtcRawAgent {
        address,
        prof: 1, // Guardian
        is_elite: 0,
        toughness: 0,
        concentration: 0,
        healing: 0,
        hitbox_width_raw: 0,
        condition: 0,
        hitbox_height_raw: 0,
        name,
    }
}

fn npc_agent(address: u64) -> EvtcRawAgent {
    let mut name = [0u8; 68];
    name[..4].copy_from_slice(b"NPCx");
    EvtcRawAgent {
        address,
        prof: 0x1234,
        is_elite: u32::MAX,
        toughness: 0,
        concentration: 0,
        healing: 0,
        hitbox_width_raw: 0,
        condition: 0,
        hitbox_height_raw: 0,
        name,
    }
}

/// 直接事件化：arc_version=(20260530,0) 且 items 先按时间排序的行为
/// 由工厂负责，这里按时间升序传入。
fn eventize(items: Vec<EvtcCombatItem>) -> Result<CombatData, ModelError> {
    let mut items = items;
    items.sort_by_key(|e| e.time);
    let log = EvtcRawLog {
        header_build: 20260530,
        revision: 1,
        id: 2,
        agents: vec![player_agent(0x1111), npc_agent(0x2222)],
        skills: vec![EvtcRawSkill {
            id: 7,
            name: "Test Skill".to_string(),
        }],
        combat_events: items,
        agent_redirection: Default::default(),
        enabled_extensions: vec![],
        map_id: -1,
        log_start_offset: 5000,
        log_end_time: 8000,
        arc_version: (20260530, 0),
        gw2_build: 0,
        stats: gw2ei_parse::ParseStats::default(),
    };
    gw2ei_model::build_combat_data(log)
}

// ===== 测试 =====

use gw2ei_model::CombatEvent;

#[test]
fn direct_crit_damage_fields() {
    let mut row = combat_row();
    row.value = 1234;
    row.result = 1; // DirectCrit
    row.is_ninety = 1;
    row.is_shields = 1;
    row.overstack_value = 999;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.direct_health_damage, 1);
    match &data.events[0] {
        CombatEvent::DirectHealthDamage(e) => {
            assert_eq!(e.skill.time, 5000);
            assert_eq!(e.skill.from, 0x1111);
            assert_eq!(e.skill.to, 0x2222);
            assert_eq!(e.skill.skill_id, 7);
            assert_eq!(e.skill.iff, Iff::Friend);
            assert!(e.skill.is_over_ninety);
            assert!(!e.skill.is_moving);
            assert_eq!(e.health_damage, 1234);
            assert_eq!(e.shield_damage, 999);
            assert!(e.has_hit);
            assert!(e.has_crit);
            assert!(!e.has_glanced);
            assert!(!e.is_blind);
            assert!(!e.is_blocked);
            assert!(!e.is_evaded);
            assert!(!e.is_absorbed);
            assert!(!e.is_life_leech);
            assert!(!e.is_not_a_damage_event);
        }
        other => panic!("expected DirectHealthDamage, got {other:?}"),
    }
}

#[test]
fn direct_blind_has_no_hit() {
    let mut row = combat_row();
    row.result = 7; // DirectBlind
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::DirectHealthDamage(e) => {
            assert!(e.is_blind);
            assert!(!e.has_hit);
            assert_eq!(e.health_damage, 0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn non_direct_buff_damage_fields() {
    let mut row = combat_row();
    row.buff = 1;
    row.buff_dmg = 555;
    row.result = 15; // BuffNotCycle
    row.is_offcycle = 1;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.non_direct_health_damage, 1);
    match &data.events[0] {
        CombatEvent::NonDirectHealthDamage(e) => {
            assert_eq!(e.health_damage, 555);
            assert!(e.has_hit);
            assert!(e.skill.against_downed); // offcycle==1
            assert!(!e.is_life_leech);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn breakbar_damage_value_scaled() {
    // 破蔑伤害：Value/10 round 1
    let mut row = combat_row();
    row.result = 10; // BreakbarDamage
    row.value = 123;
    row.buff = 0;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.breakbar_damage, 1);
    match &data.events[0] {
        CombatEvent::BreakbarDamage(e) => assert_eq!(e.value, 12.3),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn breakbar_recovery_by_generic_id_from_unknown() {
    let (_, generic) = skill_ids::dodge_and_breakbar_for_build(20260530);
    let mut row = combat_row();
    row.result = 10; // BreakbarDamage
    row.skill_id = generic;
    row.src_agent = 0; // unknown 来源
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.breakbar_recovery, 1);
}

#[test]
fn crowd_control_defiance_calculation() {
    let mut row = combat_row();
    row.result = 12; // CrowdControl
    row.value = 2500;
    row.overstack_value = 400;
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::CrowdControl(e) => {
            assert_eq!(e.duration, 2500);
            assert_eq!(e.defiance_calculation, 2900);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn no_damage_interrupt_event() {
    let mut row = combat_row();
    row.result = 5; // Interrupt
    row.is_offcycle = 1; // NoDamage 不走 NonDamageEvent 基类 → AgainstDowned 恒 false
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.no_damage_health_damage, 1);
    match &data.events[0] {
        CombatEvent::NoDamageHealthDamage(e) => {
            assert!(e.is_not_a_damage_event);
            assert!(e.has_interrupted);
            assert!(!e.has_killed);
            assert!(!e.has_downed);
            assert!(!e.skill.against_downed);
            assert_eq!(e.health_damage, 0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_apply_fields_and_initial() {
    let mut row = state_row(StateChange::BuffApply);
    row.value = 5000; // AppliedDuration
    row.pad = 0xABCD; // BuffInstance
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.buff_apply, 1);
    match &data.events[0] {
        CombatEvent::BuffApply(e) => {
            assert_eq!(e.apply.base.buff_id, 7);
            assert_eq!(e.apply.base.by, 0x1111);
            assert_eq!(e.apply.base.to, 0x2222);
            assert_eq!(e.apply.buff_instance, 0xABCD);
            assert!(!e.initial);
            assert_eq!(e.applied_duration, 5000);
            assert_eq!(e.original_applied_duration, 5000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_initial_uses_buff_dmg_as_original_duration() {
    let mut row = state_row(StateChange::BuffInitial);
    row.value = 3000;
    row.buff_dmg = 7000; // 日志开始的真实剩余时长
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::BuffApply(e) => {
            assert!(e.initial);
            assert_eq!(e.applied_duration, 3000);
            assert_eq!(e.original_applied_duration, 7000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_change_becomes_extension() {
    let mut row = state_row(StateChange::BuffChange);
    row.value = 1000; // ExtendedDuration
    row.overstack_value = 9000; // NewDuration
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.buff_extension, 1);
    match &data.events[0] {
        CombatEvent::BuffExtension(e) => {
            assert_eq!(e.extended_duration, 1000);
            assert_eq!(e.new_duration, 9000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_remove_direction_swap() {
    // AbstractBuffRemoveEvent：By=DstAgent、To=SrcAgent（方向修正）
    let mut row = state_row(StateChange::BuffRemoveSingle);
    row.buff_remove = gw2ei_parse::BuffRemove::Single;
    row.value = 500; // RemovedDuration
    row.pad = 42; // BuffInstance
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.buff_remove_single, 1);
    match &data.events[0] {
        CombatEvent::BuffRemoveSingle(e) => {
            assert_eq!(e.remove.base.by, 0x2222); // dst 原为 to
            assert_eq!(e.remove.base.to, 0x1111); // src 原为 from
            assert_eq!(e.remove.removed_duration, 500);
            assert_eq!(e.buff_instance, 42);
            // by 是已知 agent（0x2222）→ 非 overstack/natural end
            assert!(!e.overstack_or_natural_end);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_remove_single_overstack_natural_end() {
    // IFF Unknown + dst==0（无施放者）→ OverstackOrNaturalEnd
    let mut row = state_row(StateChange::BuffRemoveSingle);
    row.buff_remove = gw2ei_parse::BuffRemove::Single;
    row.dst_agent = 0;
    row.iff = Iff::Unknown;
    row.iff_raw = 2;
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::BuffRemoveSingle(e) => assert!(e.overstack_or_natural_end),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_remove_all_uses_result_and_buff_dmg() {
    let mut row = state_row(StateChange::BuffRemoveAll);
    row.value = 1000;
    row.buff_dmg = 250; // 末层时长
    row.result = 3; // RemovedStacks
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::BuffRemoveAll(e) => {
            assert_eq!(e.removed_stacks, 3);
            assert_eq!(e.last_removed_duration, 250);
            assert_eq!(e.remove.removed_duration, 1000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_stack_active_fields() {
    let mut row = state_row(StateChange::StackActive);
    row.dst_agent = 0xDEADBEEF; // BuffInstance=(uint)DstAgent
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::BuffStackActive(e) => {
            assert_eq!(e.base.by, 0); // unknown
            assert_eq!(e.base.to, 0x1111); // src
            assert_eq!(e.buff_instance, 0xDEADBEEF);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn buff_stack_deactive_fields() {
    let mut row = state_row(StateChange::StackDeactive);
    row.value = 4000; // ResetToDuration
    row.pad = 77;
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::BuffStackDeactive(e) => {
            assert_eq!(e.reset_to_duration, 4000);
            assert_eq!(e.stack.buff_instance, 77);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ===== cast 配对 =====

/// AnimationStart 行 helper。
fn anim_start(time: i64, skill: u32, result: u8, value: i32, buff_dmg: i32) -> EvtcCombatItem {
    let mut r = state_row(StateChange::AnimationStart);
    r.time = time;
    r.skill_id = skill;
    r.result = result;
    r.value = value;
    r.buff_dmg = buff_dmg;
    r
}

fn anim_stop(time: i64, skill: u32, result: u8, value: i32, buff_dmg: i32) -> EvtcCombatItem {
    let mut r = state_row(StateChange::AnimationStop);
    r.time = time;
    r.skill_id = skill;
    r.result = result;
    r.value = value;
    r.buff_dmg = buff_dmg;
    r
}

#[test]
fn cast_pair_start_end_normal() {
    // start Expected=BuffDmg(1500)，end Actual=value、Reset → Full
    let start = anim_start(5000, 7, 1, 100, 1500); // AnimationStart.Command
    let mut end = anim_stop(6500, 7, 5, 1500, 0); // AnimationStop.Ended
    end.activation = gw2ei_parse::Activation::Reset;
    let data = eventize(vec![start, end]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 1);
    match &data.events[0] {
        CombatEvent::AnimatedCast(e) => {
            assert_eq!(e.time, 5000);
            assert_eq!(e.skill_id, 7);
            assert_eq!(e.caster, 0x1111);
            assert_eq!(e.expected_duration, 1500);
            assert_eq!(e.actual_duration, 1500);
            assert_eq!(e.anim_start, gw2ei_parse::AnimationStart::Command);
            assert_eq!(e.anim_stop, gw2ei_parse::AnimationStop::Ended);
            assert_eq!(e.status, gw2ei_model::AnimationStatus::Full);
            assert_eq!(e.acceleration, 0.0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_pair_interrupted_by_cancel() {
    let start = anim_start(5000, 7, 1, 0, 2000);
    let mut end = anim_stop(6000, 7, 6, 800, 0); // Cancel
    end.activation = gw2ei_parse::Activation::Cancel;
    let data = eventize(vec![start, end]).expect("eventize");
    match &data.events[0] {
        CombatEvent::AnimatedCast(e) => {
            assert_eq!(e.status, gw2ei_model::AnimationStatus::Interrupted);
            // sanity check：|800-(6000-5000)|=200 > 150 → Actual=diff=1000
            //（AnimatedCastEvent.cs:96-100），SavedDuration=-Actual。
            assert_eq!(e.actual_duration, 1000);
            assert_eq!(e.saved_duration, -1000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_orphan_end_not_before_log_start_dropped() {
    // 孤儿 end：起算时间（end.time - actual = 1000-500=500）>= log_start(0)
    // → 丢弃（C# 仅收 Time < logStart 的动画，CombatEventFactory.cs:769-775）。
    let end = anim_stop(1000, 7, 1, 500, 0);
    let data = eventize(vec![end]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 0);
    assert_eq!(data.events.len(), 0);
}

#[test]
fn cast_orphan_end_before_log_start_kept_with_shifted_time() {
    // 孤儿 end：actual=end.Value=900，Time 前移 → 400-900=-500 < log_start(0)
    // → 保留，且事件时间落在 -500（C# AnimatedCastEvent.cs:96-104）。
    let end = anim_stop(400, 7, 1, 900, 0);
    let data = eventize(vec![end]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 1);
    match &data.events[0] {
        CombatEvent::AnimatedCast(e) => {
            assert_eq!(e.time, -500);
            assert_eq!(e.actual_duration, 900);
            assert_eq!(e.expected_duration, 900);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_end_value_kept_when_within_server_delay_of_diff() {
    // actual = end.Value = 800；|800-(5900-5000)=100| ≤ 150 → sanity check
    // 不触发，保持 end.Value（AnimatedCastEvent.cs:92-96）。
    let start = anim_start(5000, 7, 1, 0, 800);
    let mut end = anim_stop(5900, 7, 5, 800, 0);
    end.activation = gw2ei_parse::Activation::Reset;
    let data = eventize(vec![start, end]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 1);
    match &data.events[0] {
        CombatEvent::AnimatedCast(e) => {
            assert_eq!(e.time, 5000);
            assert_eq!(e.actual_duration, 800);
            assert_eq!(e.expected_duration, 800);
            assert_eq!(e.status, gw2ei_model::AnimationStatus::Full);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_orphan_start_preserved_with_expected_duration() {
    // 孤儿 start（无 end）：Actual=Expected（1500）且未超 logEnd 不截断。
    let start = anim_start(5000, 7, 1, 0, 1500);
    let data = eventize(vec![start]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 1);
    match &data.events[0] {
        CombatEvent::AnimatedCast(e) => {
            assert_eq!(e.expected_duration, 1500);
            assert_eq!(e.actual_duration, 1500);
            assert_eq!(e.time, 5000);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_player_short_duration_dropped() {
    // 玩家(0x1111) Actual<=1ms 丢弃。
    let start = anim_start(5000, 7, 1, 0, 1000);
    let mut end = anim_stop(5000, 7, 1, 1, 0); // actual=1ms
    end.activation = gw2ei_parse::Activation::Minimum;
    let data = eventize(vec![start, end]).expect("eventize");
    assert_eq!(data.counts.animated_cast, 0);
}

#[test]
fn cast_emote_dispatch_by_special_id() {
    // skill 23303 → Emote；emote_id = start.OverstackValue
    let mut start = anim_start(5000, skill_ids::ARC_GENERIC_EMOTE, 7, 0, 1000);
    start.overstack_value = 0xCAFE;
    let mut end = anim_stop(5500, skill_ids::ARC_GENERIC_EMOTE, 7, 500, 0);
    end.activation = gw2ei_parse::Activation::Reset;
    let data = eventize(vec![start, end]).expect("eventize");
    assert_eq!(data.counts.emote, 1);
    match &data.events[0] {
        CombatEvent::Emote(e) => {
            assert_eq!(e.emote_id, 0xCAFE);
            assert_eq!(e.base.anim_start, gw2ei_parse::AnimationStart::Emote);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cast_bundle_pickup_breaks_caster() {
    let mut start = anim_start(5000, skill_ids::ARC_GENERIC_PICK_UP, 8, 0, 0);
    start.overstack_value = 0xBEEF;
    let mut end = anim_stop(5200, skill_ids::ARC_GENERIC_PICK_UP, 25, 200, 0);
    end.activation = gw2ei_parse::Activation::Reset;
    let data = eventize(vec![start, end]).expect("eventize");
    assert_eq!(data.counts.bundle_pick_up, 1);
    match &data.events[0] {
        CombatEvent::BundlePickUp(e) => {
            assert_eq!(e.base.caster, 0); // caster 断开
            assert_eq!(e.bundle_id, 0xBEEF);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ===== 状态事件 =====

#[test]
fn enter_combat_spec_and_subgroup() {
    // Value=prof、BuffDmg=elite → Guardian（本地表 elite==0）
    let mut row = state_row(StateChange::EnterCombat);
    row.dst_agent = 7; // Subgroup
    row.value = 1; // prof Guardian
    row.buff_dmg = 0; // elite 0 → 基础职业
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.enter_combat, 1);
    match &data.events[0] {
        CombatEvent::EnterCombat(e) => {
            assert_eq!(e.subgroup, 7);
            assert_eq!(e.spec, gw2ei_model::Spec::Guardian);
            assert_eq!(e.spec.base_spec(), gw2ei_model::Spec::Guardian);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn position_unpacks_xy_from_dst_and_z_from_value() {
    let mut row = state_row(StateChange::Position);
    // x=1.5f32, y=-2.25f32 打包进 DstAgent 低 8B
    let x = 1.5f32.to_le_bytes();
    let y = (-2.25f32).to_le_bytes();
    let mut packed = [0u8; 8];
    packed[0..4].copy_from_slice(&x);
    packed[4..8].copy_from_slice(&y);
    row.dst_agent = u64::from_le_bytes(packed);
    row.value = 100.0f32.to_bits() as i32; // z
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.movement, 1);
    match &data.events[0] {
        CombatEvent::Position(e) => {
            assert_eq!(e.x, 1.5);
            assert_eq!(e.y, -2.25);
            assert_eq!(e.z, 100.0);
            assert_eq!(e.base.src, 0x1111);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn movement_nan_dropped() {
    let mut row = state_row(StateChange::Velocity);
    row.value = f32::NAN.to_bits() as i32;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.movement, 0);
}

#[test]
fn targetable_dedup() {
    let mut a = state_row(StateChange::Targetable);
    a.time = 5000;
    a.dst_agent = 1;
    let mut b = state_row(StateChange::Targetable);
    b.time = 5100;
    b.dst_agent = 1; // 与上次同值 → 去重
    let mut c = state_row(StateChange::Targetable);
    c.time = 5200;
    c.dst_agent = 0;
    let data = eventize(vec![a, b, c]).expect("eventize");
    assert_eq!(data.counts.targetable, 2);
}

// ===== metadata =====

#[test]
fn buff_info_and_formula_merge_by_id() {
    let mut info = state_row(StateChange::BuffInfo);
    info.skill_id = 100;
    info.is_flanking = 1; // ProbablyInvul
    info.overstack_value = 5000; // DurationCap
    let mut formula = state_row(StateChange::BuffFormula);
    formula.skill_id = 100;
    // 44B payload 前 8B（Time 字段位型）承载 Type/ByteAttr1 两个 float。
    let mut buf = [0u8; 44];
    buf[0..4].copy_from_slice(&1.0f32.to_le_bytes()); // Type
    buf[4..8].copy_from_slice(&2.0f32.to_le_bytes()); // ByteAttr1
    formula.time = i64::from_le_bytes(buf[0..8].try_into().expect("8 bytes"));
    let data = eventize(vec![info, formula]).expect("eventize");
    let entry = data
        .metadata
        .buff_info_by_id
        .get(&100)
        .expect("buff info merged");
    assert_eq!(entry.buff_id, 100);
    let info_row = entry.info.as_ref().expect("info row present");
    assert!(info_row.probably_invul);
    assert_eq!(info_row.duration_cap, 5000);
    assert_eq!(entry.formulas.len(), 1);
    // formula_type 从打包区首个 float 读出 = 1
    assert_eq!(entry.formulas[0].formula_type, 1);
    // byte_attr1 来自打包区第 2 个 float(偏移 4)=2（浮点表示 2.0）
    assert_eq!(entry.formulas[0].byte_attr1, 2);
}

#[test]
fn guid_events_build_128bit_hex() {
    let mut row = state_row(StateChange::IDToGUID);
    row.overstack_value = 2; // ContentLocal.Skill
    row.skill_id = 0x1234;
    row.src_agent = 0x0102030405060708;
    row.dst_agent = 0x1112131415161718;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.metadata_guid, 1);
    // GUID 行级事件在 metadata 桶（C# 的 metaDataEvents GUID 字典域）。
    assert_eq!(data.events.len(), 0);
    assert_eq!(data.metadata.id_to_guid.len(), 1);
    match &data.metadata.id_to_guid[0] {
        CombatEvent::GuidSkill(e) => {
            assert_eq!(e.content_id, 0x1234);
            assert_eq!(
                e.guid_hex(),
                "08070605040302011817161514131211"
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn health_update_clamps_percent() {
    let mut row = state_row(StateChange::HealthUpdate);
    row.dst_agent = 12345; // 123.45%
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::HealthUpdate(e) => assert_eq!(e.percent, 100.0),
        other => panic!("unexpected {other:?}"),
    }
    let mut row = state_row(StateChange::HealthUpdate);
    row.dst_agent = 5432; // 54.32%
    let data = eventize(vec![row]).expect("eventize");
    match &data.events[0] {
        CombatEvent::HealthUpdate(e) => assert_eq!(e.percent, 54.32),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn weapon_swap_event() {
    let mut row = state_row(StateChange::WeaponSwap);
    row.dst_agent = 4; // FirstLandSet
    row.value = 5; // 上一套
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.weapon_swap, 1);
    match &data.events[0] {
        CombatEvent::WeaponSwap(e) => {
            assert_eq!(e.swapped_to, 4);
            assert_eq!(e.swapped_from, 5);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn unsupported_marker_counted() {
    let row = state_row(StateChange::Marker);
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.events.len(), 0);
    assert_eq!(data.unsupported[&gw2ei_model::UnsupportedEventKind::Marker], 1);
}

#[test]
fn old_build_rejected() {
    // build < 20260501 → UnsupportedArcBuild
    let mut items = vec![combat_row()];
    items.sort_by_key(|e| e.time);
    let log = EvtcRawLog {
        header_build: 20250430,
        revision: 1,
        id: 2,
        agents: vec![player_agent(0x1111)],
        skills: vec![],
        combat_events: items,
        agent_redirection: Default::default(),
        enabled_extensions: vec![],
        map_id: -1,
        log_start_offset: 5000,
        log_end_time: 8000,
        arc_version: (20250430, 0),
        gw2_build: 0,
        stats: gw2ei_parse::ParseStats::default(),
    };
    match gw2ei_model::build_combat_data(log) {
        Err(ModelError::UnsupportedArcBuild { build }) => assert_eq!(build, 20250430),
        other => panic!("expected UnsupportedArcBuild, got {other:?}"),
    }
}

#[test]
fn stun_break_direction_and_duration() {
    let mut row = state_row(StateChange::StunBreak);
    row.value = 3000;
    row.is_offcycle = 1;
    let data = eventize(vec![row]).expect("eventize");
    assert_eq!(data.counts.stun_break, 1);
    match &data.events[0] {
        CombatEvent::StunBreak(e) => {
            assert_eq!(e.skill.from, 0); // unknown
            assert_eq!(e.skill.to, 0x1111); // 原 src
            assert_eq!(e.remaining_duration, 3000);
            assert!(e.skill.against_downed);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ===== P4:WvWObjectiveStatus 聚合(首现序 + 同 key owners 追加)=====

#[test]
fn wvw_objective_status_first_seen_aggregation() {
    let mk = |t: i64, map: i32, obj: u32, team: i32| {
        let mut r = state_row(StateChange::WvWObjectiveStatus);
        r.time = t;
        r.value = map; // MapID
        r.skill_id = obj; // ObjectiveID
        r.buff_dmg = team; // TeamID
        r
    };
    // 首现序:obj34 → obj52 → obj34(同 key 追加 owners;不合并)
    let data = eventize(vec![
        mk(1000, 96, 34, 707),
        mk(2000, 96, 52, 433),
        mk(3000, 96, 34, 2767),
    ])
    .expect("eventize");
    let objs = &data.metadata.wvw_objective_statuses;
    assert_eq!(objs.len(), 2, "first-seen keys: 34 then 52");
    assert_eq!(objs[0].map_id, 96);
    assert_eq!(objs[0].objective_id, 34);
    assert_eq!(objs[0].owners, vec![(707, 1000), (2767, 3000)]);
    assert_eq!(objs[1].map_id, 96);
    assert_eq!(objs[1].objective_id, 52);
    assert_eq!(objs[1].owners, vec![(433, 2000)]);
}
