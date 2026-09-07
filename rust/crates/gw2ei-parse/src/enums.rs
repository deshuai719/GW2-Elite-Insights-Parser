//! ArcDPS 枚举与版本常量，完整移植 `GW2EIEvtcParser/ParserHelpers/ArcDPSEnums.cs`
//! （upstream 快照 `3b7278f9b`）。解析层与事件化层共用。
//!
//! `from_byte` 折叠语义对齐 C# 的 `Get*` 辅助方法：值落在 `[0, Unknown)` 区间内
//! 直接 cast，否则折叠为 `Unknown`（C# `ArcDPSEnums.cs` 各 `Get*` 的
//! `bt < (byte)Unknown` 判定，Unknown 是枚举内的显式成员而非表外哨兵）。
//!
//! `csharp_name` 输出 C# `Enum.ToString()` 的变体名；输入已是折叠后值
//! （C# 的 `CombatItem` 构造即经 `Get*` 折叠、原始字节不可观测），表外数字
//! 分支只作防御——真实日志的该字段只会落到定义值或 `Unknown` 上。

macro_rules! arc_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $value:expr),+ $(,)? }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant = $value,)+
        }

        impl $name {
            /// 对齐 C# `Get*`：表外值折叠为 `Unknown` 成员。
            pub fn from_byte(byte: u8) -> Self {
                match byte {
                    $($value => Self::$variant,)+
                    _ => Self::Unknown,
                }
            }

            /// C# `Enum.ToString()` 变体名（变体名 = C# 成员名）。
            pub fn csharp_name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                }
            }
        }
    };
}

// ===== ArcDPS 版本分界常量（ArcDPSEnums.cs:8-46）=====

/// ArcDPS 版本分界常量。事件行语义（buff apply/remove 是否自带 state change、
/// cast 起止行形态、DamageResult 世代）按这些 YYYYMMDD 常量分支。
pub mod arc_builds {
    pub const START_OF_LIFE: i32 = i32::MIN;
    /// 20210529
    pub const PROPER_CONFUSION_DAMAGE_SIMULATION: i32 = 20210529;
    /// 20210800
    pub const SCORING_SYSTEM_CHANGE: i32 = 20210800;
    /// 20210923
    pub const DIRECT_X11_UPDATE: i32 = 20210923;
    /// 20220304
    pub const INTERNAL_SKILL_IDS_CHANGE: i32 = 20220304;
    /// 20220308
    pub const BUFF_ATTR_FLAT_INC_REMOVED: i32 = 20220308;
    /// 20220709
    pub const FUNCTIONAL_ID_TO_GUID_EVENTS: i32 = 20220709;
    /// 20221111
    pub const NEW_LOG_START: i32 = 20221111;
    /// 20230718
    pub const EFFECT2_EVENTS: i32 = 20230718;
    /// 20230719
    pub const FUNCTIONAL_EFFECT2_EVENTS: i32 = 20230719;
    /// 20230905
    pub const BUFF_EXTENSION_BROKEN: i32 = 20230905;
    /// 20231107
    pub const BUFF_EXTENSION_OVERSTACK_VALUE_CHANGED: i32 = 20231107;
    /// 20231110
    pub const LINGERING_AGENTS: i32 = 20231110;
    /// 20240211
    pub const REMOVED_DURATION_FOR_INFINITE_DURATION_STACKS_CHANGED: i32 = 20240211;
    /// 20240418
    pub const NEW_MARKER_EVENT_BEHAVIOR: i32 = 20240418;
    /// 20240529
    pub const LAST90_BEFORE_DOWN_RETIRED: i32 = 20240529;
    /// 20240609
    pub const STACK_TYPE0_ACTIVE_CHANGE: i32 = 20240609;
    /// 20240612
    pub const TEAM_CHANGE_ON_DESPAWN: i32 = 20240612;
    /// 20240627
    pub const WEAPON_SWAP_VALUE_IS_PREVIOUS_CROWD_CONTROL_EVENTS_GLIDER_EVENTS: i32 = 20240627;
    /// 20240709
    pub const MOVEMENT_SKILL_DETECTION: i32 = 20240709;
    /// 20240716
    pub const EI_CAN_DO_MANUAL_BUFF_ATTRIBUTES: i32 = 20240716;
    /// 20241030
    pub const EXTRA_DATA_IN_GUID_EVENTS: i32 = 20241030;
    /// 20250315
    pub const LOG_START_LOG_END_PER_COMBAT_SEQUENCE_ON_INSTANCE_LOGS: i32 = 20250315;
    /// 20250428
    pub const SPECIES_SKILL_GUIDS: i32 = 20250428;
    /// 20250525
    pub const MISSILES_INTRODUCED: i32 = 20250525;
    /// 20250913
    pub const BUFF_FORMULA_ORIGINAL_ATTRIBUTE: i32 = 20250913;
    /// 20260318：emote / gadget interact / bundle pick-up 三条动画作为独立技能 ID 行族。
    pub const EMOTE_AND_GADGET_INTERACTION_ADDED: i32 = 20260318;
    /// 20260430：cast 起止行从 `Combat + Activation` 改为 `StateChange.AnimationStart/Stop`。
    pub const ANIMATION_AS_STATE_CHANGES: i32 = 20260430;
    /// 20260501：buff apply/remove 行自带 state change；`DamageResult` 取代 `ConditionResult`。
    pub const BUFF_APPLIES_AND_REMOVES_AS_STATE_CHANGES: i32 = 20260501;
    /// 20260501：同 20260501。
    pub const RESULT_ENUM_REWORK: i32 = 20260501;
    /// 20260522
    pub const VISIBILITY_IN_TARGETABLE_STATE_CHANGE: i32 = 20260522;
    /// 20260527
    pub const VISIBILITY_ON_STATE_CHANGE: i32 = 20260527;
    /// 20260602
    pub const GADGET_CAPTURES_ADDED: i32 = 20260602;
    /// 上限哨兵。
    pub const END_OF_LIFE: i32 = i32::MAX;
}

arc_enum! {
    /// Skill 轴事件 start/end/cancel/reset 的来源（ArcDPSEnums.cs:105-116）。
    Activation {
        None = 0, // not used - not this kind of event
        Normal = 1, // started skill/animation activation
        Quickness = 2, // unused as of nov 5 2019
        Minimum = 3, // stopped skill activation with reaching tooltip time
        Cancel = 4, // stopped skill activation without reaching tooltip time
        Reset = 5, // animation completed fully
        NoData = 6, // Treat it as CancelFire
        Unknown = 7,
    }
}

arc_enum! {
    /// 新世代（>= 20260430）cast start 行的类型字节（ArcDPSEnums.cs:122-140）。
    AnimationStart {
        None = 0,
        Command = 1,
        Dodge = 2,
        StowDraw = 3,
        MoveSkill = 4,
        MotionSkill = 5,
        GadgetInteract = 6,
        Emote = 7,
        PickUp = 8,
        Unknown = 9,
    }
}

arc_enum! {
    /// 新世代 cast end 行的类型字节（ArcDPSEnums.cs:141-176）。
    AnimationStop {
        None = 0,
        Instant = 1,
        MultiDefunc = 2,
        Transition = 3,
        Partial = 4,
        Ended = 5,
        Cancel = 6,
        StowDraw = 7,
        Interrupt = 8,
        Death = 9,
        Downed = 10,
        CrowdControl = 11,
        Command = 12,
        MotionSkill = 13,
        MoveDodge = 14,
        MotionSkillViaReset = 15,
        MoveSkill = 16,
        MovePos = 17,
        Any = 18,
        GadgetViaReset = 19,
        ManualExpiry = 20,
        Despawn = 21,
        ReturnControl = 22,
        Ready = 23,
        Invisible = 24,
        PickUp = 25,
        Unknown = 26,
    }
}

arc_enum! {
    /// Buff remove 语义（ArcDPSEnums.cs:178-191）。
    BuffRemove {
        None = 0,
        All = 1,
        Single = 2,
        Manual = 3,
        Unknown = 4,
    }
}

arc_enum! {
    /// Buff 周期语义（ArcDPSEnums.cs:195-209）。Retired as of 20260501。
    #[allow(non_camel_case_types)] // C# 成员名含 NotCycle_* 下划线
    BuffCycle {
        Cycle = 0, // damage happened on tick timer
        NotCycle = 1, // damage happened outside tick timer (resistable)
        NotCycle_NoResit = 2, // BEFORE MAY 2021: the others were lumped here, now retired
        NotCycle_DamageToTargetOnHit = 3, // damage happened to target on hiting target
        NotCycle_DamageToSourceOnHit = 4, // damage happened to source on hiting target
        NotCycle_DamageToTargetOnStackRemove = 5, // damage happened to target on source losing a stack
        Unknown = 6,
    }
}

arc_enum! {
    /// 伤害结果（ArcDPSEnums.cs:213-241）。`ResultEnumRework` 世代的值域。
    #[allow(non_camel_case_types)] // C# 成员名含 BuffNotCycle_* 下划线
    DamageResult {
        DirectNormal = 0,
        DirectCrit = 1,
        DirectGlance = 2,
        DirectBlock = 3,
        DirectEvade = 4,
        Interrupt = 5,
        DirectOrBuffAbsorb = 6,
        DirectBlind = 7,
        KillingBlow = 8,
        Downed = 9,
        BreakbarDamage = 10,
        Activation = 11,
        CrowdControl = 12,
        DirectOrBuffInvert = 13,
        BuffCycle = 14, // damage happened on tick timer
        BuffNotCycle = 15, // damage happened outside tick timer (resistable)
        BuffNotCycle_DamageToTargetOnHit = 16, // damage happened to target on hiting target
        BuffNotCycle_DamageToSourceOnHit = 17, // damage happened to source on hiting target
        BuffNotCycle_DamageToTargetOnStackRemove = 18, // damage happened to target on source losing a stack
        Unknown = 19,
    }
}

arc_enum! {
    /// 旧世代 condition 结果（ArcDPSEnums.cs:244-257）。Retired as of 20260501。
    ConditionResult {
        ExpectedToHit = 0,
        InvulByBuff = 1,
        InvulByPlayerSkill1 = 2,
        InvulByPlayerSkill2 = 3,
        InvulByPlayerSkill3 = 4,
        Unknown = 5,
    }
}

arc_enum! {
    /// StateChange（ArcDPSEnums.cs:260-355）。
    #[allow(non_camel_case_types)] // C# 成员名含 Effect_45/Effect_51 等，csharp_name 需逐字对齐
    StateChange {
        Combat = 0,
        EnterCombat = 1,
        ExitCombat = 2,
        ChangeUp = 3,
        ChangeDead = 4,
        ChangeDown = 5,
        Spawn = 6,
        Despawn = 7,
        HealthUpdate = 8,
        SquadCombatStart = 9,
        SquadCombatEnd = 10,
        WeaponSwap = 11,
        MaxHealthUpdate = 12,
        PointOfView = 13,
        Language = 14,
        GWBuild = 15,
        ShardID = 16,
        Reward = 17,
        BuffInitial = 18,
        Position = 19,
        Velocity = 20,
        Rotation = 21,
        TeamChange = 22,
        AttackTarget = 23,
        Targetable = 24,
        MapID = 25,
        ReplInfo = 26,
        StackActive = 27,
        StackDeactive = 28, // Formerly as StackReset
        Guild = 29,
        BuffInfo = 30,
        BuffFormula = 31,
        SkillInfo = 32,
        SkillTiming = 33,
        BreakbarState = 34,
        BreakbarPercent = 35,
        Integrity = 36,
        Marker = 37,
        BarrierUpdate = 38,
        StatReset = 39,
        Extension = 40,
        APIDelayed = 41,
        InstanceStart = 42,
        TickRate = 43,
        Last90BeforeDown = 44,
        Effect_45 = 45,
        IDToGUID = 46,
        LogNPCUpdate = 47,
        Idle = 48,
        ExtensionCombat = 49,
        FractalScale = 50,
        Effect_51 = 51,
        RuleSet = 52,
        SquadMarker = 53,
        ArcBuild = 54,
        Glider = 55,
        StunBreak = 56,
        MissileCreate = 57,
        MissileLaunch = 58,
        MissileRemove = 59,
        EffectGroundCreate = 60,
        EffectGroundRemove = 61,
        EffectAgentCreate = 62,
        EffectAgentRemove = 63,
        AgentChange = 64,
        MapChange = 65,
        EarlyExit = 66,
        AnimationStart = 67,
        AnimationStop = 68,
        BuffApply = 69,
        BuffChange = 70, // Extension
        BuffRemoveSingle = 71, // Single or Manual
        BuffRemoveAll = 72,
        Transformation = 73,
        WvWTeams = 74,
        WvWObjectiveStatus = 75,
        StealthChange = 76,
        GadgetAnimation = 77,
        GadgetNameVisible = 78,
        EffectMissileCreate = 79,
        GadgetCaptureOutlineShow = 80,
        GadgetCaptureSplitPercent = 81,
        GadgetCaptureOutlineHide = 82,
        GadgetCaptureOutlinePoint = 83,
        Tick = 84,
        Teleport = 85,
        Jump = 86, // Ignore for now
        Unknown = 87,
    }
}

arc_enum! {
    /// 日志类型（ArcDPSEnums.cs:358-369）。
    LogType {
        None = 0,
        Auto = 1,
        Map = 2,
        Generic = 3,
        Unknown = 4,
    }
}

impl LogType {
    /// 对齐 `GetLogType(int)`（ArcDPSEnums.cs:366-370）：C# 以 **i32** 与
    /// `< (int)Unknown` 比较（int 形参），通过后按底层 byte 截断 cast。
    /// 折叠分支与 `from_byte` 相同；唯一语义差异在「负值且低 8 位 ≥ Unknown」
    /// 的不可达输入上：C# 会 cast 出未定义字节值，此处折叠为 `Unknown`。
    pub fn from_csharp_int(value: i32) -> Self {
        if value < Self::Unknown as i32 {
            Self::from_byte(value as u8)
        } else {
            Self::Unknown
        }
    }
}

arc_enum! {
    /// 破蔑条状态（ArcDPSEnums.cs:372-383）。
    BreakbarState {
        Active = 0,
        Recover = 1,
        Immune = 2,
        None = 3,
        Unknown = 4,
    }
}

impl BreakbarState {
    /// 对齐 `GetBreakbarState(int)`（ArcDPSEnums.cs:380-384）：C# 形参是
    /// `evtcItem.Value`（int），判定与 cast 语义同 `LogType::from_csharp_int`。
    pub fn from_csharp_int(value: i32) -> Self {
        if value < Self::Unknown as i32 {
            Self::from_byte(value as u8)
        } else {
            Self::Unknown
        }
    }
}

arc_enum! {
    /// Buff 堆叠类型（ArcDPSEnums.cs:388-401）。
    BuffStackType {
        StackingConditionalLoss = 0, // same as Stacking but individual stacks can be removed
        Queue = 1,
        StackingUniquePerSrc = 2, // same as Stacking but individual stacks can be extended and one src can only have one stack active at a time
        Regeneration = 3,
        Stacking = 4,
        Force = 5,
        Unknown = 6,
    }
}

arc_enum! {
    /// GUID 内容的本地类型（ArcDPSEnums.cs:604-618）。
    ContentLocal {
        Effect = 0,
        Marker = 1,
        Skill = 2,
        Species = 3,
        Team = 4,
        Emote = 5,
        Transformation = 6,
        Unknown = 7,
    }
}

arc_enum! {
    /// 敌我标识（ArcDPSEnums.cs:622-633）。
    Iff {
        Friend = 0,
        Foe = 1,
        Unknown = 2,
    }
}

arc_enum! {
    /// 语言（ArcDPSEnums.cs:637-652）。
    Language {
        English = 0,
        Missing = 1,
        French = 2,
        German = 3,
        Spanish = 4,
        Chinese = 5,
        Unknown = 6,
    }
}

arc_enum! {
    /// 小队标记索引（ArcDPSEnums.cs:585-601）。
    SquadMarkerIndex {
        Arrow = 0,
        Circle = 1,
        Heart = 2,
        Square = 3,
        Star = 4,
        Swirl = 5,
        Triangle = 6,
        X = 7,
        Unknown = 8,
    }
}

arc_enum! {
    /// SkillTiming 动作（ArcDPSEnums.cs:572-581）。表外值折叠到
    /// `Unknown = 255`（C# 用 `Enum.IsDefined` 判定，只有 4/5 是定义值）。
    SkillAction {
        EffectHappened = 4,
        AnimationCompleted = 5,
        Unknown = 255,
    }
}

// 语言转显示名（C# LanguageToString, ArcDPSEnums.cs:655-667）。
impl Language {
    pub fn display_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Missing => "Missing",
            Language::French => "French",
            Language::German => "German",
            Language::Spanish => "Spanish",
            Language::Chinese => "Chinese",
            Language::Unknown => "Unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_change_values() {
        assert_eq!(StateChange::Combat as u8, 0);
        assert_eq!(StateChange::AnimationStart as u8, 67);
        assert_eq!(StateChange::BuffApply as u8, 69);
        assert_eq!(StateChange::BuffChange as u8, 70);
        assert_eq!(StateChange::BuffRemoveSingle as u8, 71);
        assert_eq!(StateChange::BuffRemoveAll as u8, 72);
        assert_eq!(StateChange::Tick as u8, 84);
        assert_eq!(StateChange::Teleport as u8, 85);
        assert_eq!(StateChange::Jump as u8, 86);
        assert_eq!(StateChange::Unknown as u8, 87);
    }

    #[test]
    fn from_byte_folds_out_of_range() {
        // 定义值直通；表外折叠 Unknown（C# Get* 的 bt < Unknown 判定）。
        assert_eq!(StateChange::from_byte(0), StateChange::Combat);
        assert_eq!(StateChange::from_byte(86), StateChange::Jump);
        assert_eq!(StateChange::from_byte(87), StateChange::Unknown);
        assert_eq!(StateChange::from_byte(200), StateChange::Unknown);
        assert_eq!(Iff::from_byte(0), Iff::Friend);
        assert_eq!(Iff::from_byte(2), Iff::Unknown);
        assert_eq!(Activation::from_byte(255), Activation::Unknown);
        // SkillAction 表外折叠目标值是 255 而非枚举末位。
        assert_eq!(SkillAction::from_byte(4), SkillAction::EffectHappened);
        assert_eq!(SkillAction::from_byte(0), SkillAction::Unknown);
    }

    #[test]
    fn from_csharp_int_matches_get_log_type_and_get_breakbar_state() {
        // C# GetLogType(int)/GetBreakbarState(int)（ArcDPSEnums.cs:366,380）：
        // 定义值直通、≥ Unknown 折叠、i32 判定。
        assert_eq!(LogType::from_csharp_int(0), LogType::None);
        assert_eq!(LogType::from_csharp_int(3), LogType::Generic);
        assert_eq!(LogType::from_csharp_int(4), LogType::Unknown);
        assert_eq!(LogType::from_csharp_int(0x102), LogType::Unknown); // (int) 截断后 258
        assert_eq!(LogType::from_csharp_int(-1), LogType::Unknown); // cast 分支字节 255，表外
        assert_eq!(BreakbarState::from_csharp_int(0), BreakbarState::Active);
        assert_eq!(BreakbarState::from_csharp_int(3), BreakbarState::None);
        assert_eq!(BreakbarState::from_csharp_int(4), BreakbarState::Unknown);
        assert_eq!(BreakbarState::from_csharp_int(-300), BreakbarState::Unknown);
    }

    #[test]
    fn csharp_names() {
        assert_eq!(StateChange::SquadCombatStart.csharp_name(), "SquadCombatStart");
        assert_eq!(StateChange::Effect_45.csharp_name(), "Effect_45");
        assert_eq!(StateChange::Unknown.csharp_name(), "Unknown");
        assert_eq!(Iff::Foe.csharp_name(), "Foe");
        assert_eq!(BuffRemove::Manual.csharp_name(), "Manual");
    }

    #[test]
    fn build_constants() {
        assert_eq!(arc_builds::ANIMATION_AS_STATE_CHANGES, 20260430);
        assert_eq!(arc_builds::BUFF_APPLIES_AND_REMOVES_AS_STATE_CHANGES, 20260501);
        assert_eq!(arc_builds::RESULT_ENUM_REWORK, 20260501);
        assert_eq!(arc_builds::EMOTE_AND_GADGET_INTERACTION_ADDED, 20260318);
        assert_eq!(arc_builds::VISIBILITY_ON_STATE_CHANGE, 20260527);
    }
}
