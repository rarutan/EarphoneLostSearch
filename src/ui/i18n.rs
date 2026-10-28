use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// 内蔵UIテキスト。標準言語は実行ファイルに含め、外部言語パックは将来この層へ
/// 追加できるように、表示側が直接日本語リテラルへ依存しない構成にする。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum Language {
    Japanese,
    English,
}

impl Default for Language {
    fn default() -> Self {
        Self::Japanese
    }
}

#[derive(Clone, Copy)]
pub struct Texts {
    pub language: Language,
}

static SPECIES_NAMES_JA: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("../../assets/text/species_names_ja.txt")
        .lines()
        .collect()
});
static SPECIES_NAMES_EN: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("../../assets/text/species_names_en.txt")
        .lines()
        .collect()
});
static NATURE_NAMES_JA: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("../../assets/text/natures_ja.txt")
        .lines()
        .collect()
});
static NATURE_NAMES_EN: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("../../assets/text/natures_en.txt")
        .lines()
        .collect()
});

macro_rules! text_method {
    ($name:ident, $ja:literal, $en:literal) => {
        pub fn $name(self) -> &'static str {
            match self.language {
                Language::Japanese => $ja,
                Language::English => $en,
            }
        }
    };
}

fn is_japanese_character(character: char) -> bool {
    matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x9fff | 0xff66..=0xff9d)
}

#[allow(dead_code)]
impl Texts {
    pub const fn new(language: Language) -> Self {
        Self { language }
    }

    pub fn language_name(self) -> &'static str {
        match self.language {
            Language::Japanese => "日本語",
            Language::English => "English",
        }
    }

    pub fn fixed_rng_category(self, category: &'static str) -> &'static str {
        if self.language == Language::English {
            return if category == "配達員" {
                "Deliveryman"
            } else {
                category
            };
        }

        match category {
            "-" => "-",
            "Legendary" => "伝説",
            "Alola Legendary" => "アローラの伝説",
            "Kanto Legendary" => "カントーの伝説",
            "Johto Legendary" => "ジョウトの伝説",
            "Hoenn Legendary" => "ホウエンの伝説",
            "Sinnoh Legendary" => "シンオウの伝説",
            "Unova Legendary" => "イッシュの伝説",
            "Kalos Legendary" => "カロスの伝説",
            "Ultra Space Wilds" => "ウルトラホール",
            "Totem" => "ぬしポケモン",
            "Starters" => "御三家",
            "Normal Stationary" => "通常固定",
            "In-Game Gift" => "ゲーム内配布",
            "Fossils" => "化石",
            "In-Game Trade" => "ゲーム内交換",
            "UB (First Encounter)" => "UB（初回遭遇）",
            "Poke Pelago" => "ポケリゾート",
            "配達員" => "配達員",
            _ => category,
        }
    }

    pub fn app_title(self) -> &'static str {
        "EarphoneLostSearch"
    }

    /// Resolve a fixed Japanese UI label at the presentation boundary.
    /// Unknown values intentionally fall back to the source text so technical
    /// identifiers and caller-owned text are not silently changed.
    pub fn ui_label(self, japanese: &'static str) -> &'static str {
        if self.language == Language::Japanese {
            return japanese;
        }
        match japanese {
            "Timeleap検索" => "Timeleap Search",
            "カテゴリー" => "Category",
            "配達員" => "Deliveryman",
            "配達員設定" => "Deliveryman settings",
            "配達員個体は配達員設定タブで指定" => {
                "Configure the Deliveryman Pokémon in the Deliveryman tab."
            }
            "Timeleap方式" | "方式" => "Timeleap method",
            "会話送り" => "Dialogue advance",
            "メニュー閉じ（男の子）" => "Menu close (Boy)",
            "メニュー閉じ（女の子）" => "Menu close (Girl)",
            "検索" => "Search",
            "生成開始Frame" => "Generation start frame",
            "Frame1まで" => "Until Frame1",
            "計算中…" => "Calculating...",
            "時間①" => "Time 1",
            "時間②" => "Time 2",
            "ずれ" => "Drift",
            "実際に欲しい個体が生成されるFrameを指定します。Consider Delayを使う場合は、A入力Frameとは異なります。" => {
                "Enter the frame that generates the Pokémon you want. With Consider Delay enabled, this differs from the A-input frame."
            }
            "Consider Delay" => "Consider Delay",
            "No Dex" => "No Dex",
            "そのままエンカウント" => "Direct encounter",
            "F +4" => "F +4",
            "ズレ検索" => "Drift search",
            "猶予" => "Streak",
            "猶予①" => "Streak 1",
            "猶予②" => "Streak 2",
            "F以上" => "F or more",
            "Initial Seed・StartingFrame・NPC数・Frame1とTargetFrameを入力すると検索できます。" => {
                "Enter the Initial Seed, StartingFrame, NPC count, Frame1, and TargetFrame to search."
            }
            "初期SEED:" => "Initial SEED:",
            "StartingFrame:" => "StartingFrame:",
            "NPC数:" => "NPC count:",
            "Frame1:" => "Frame1:",
            "Reset" => "Reset",
            "Frame1検索" => "Frame1 Search",
            "TargetFrame検索" => "TargetFrame Search",
            "設定" => "Settings",
            "範囲指定" => "Range",
            "～" => "to",
            "時間①秒数" => "Time 1 (seconds)",
            "Frame1" => "Frame1",
            "秒" => "s",
            "Frame1とTargetFrameをセット" => "Set Frame1 and TargetFrame",
            "TargetFrameをセット" => "Set TargetFrame",
            "Offsetをセット（計算不可）" => "Set offset (unavailable)",
            "実際にタイムリープを行うFrameです。" => {
                "The frame at which to perform the Timeleap."
            }
            "同じ生成開始Frameにたどり着けるTimelineへ移動できるFrame1が、前後にどれくらい連続しているかを2F単位で表示します。" => {
                "Shows, in 2-frame increments, how far consecutive Frame1 values can move to timelines that reach the same generation start frame."
            }
            "同じ生成開始Frameにたどり着けるTargetFrameが、前後にどれくらい連続しているかを2F単位で表示します。" => {
                "Shows, in 2-frame increments, how far consecutive TargetFrames reach the same generation start frame."
            }
            "実際にエンカウントするためAボタンを押すFrameです。" => {
                "The frame on which to press A to encounter the Pokémon."
            }
            "現在時刻からFrame1までの残り秒数です。内部で使われているタイムラインと、入力されている情報が違う場合、StartingFrameからFrame1までの秒数になります。" => {
                "Time remaining until Frame1. If the active timeline differs from the entered seed, StartingFrame, or NPC count, this is calculated from StartingFrame."
            }
            "Frame1からTargetFrameまでの時間です。" => "Time from Frame1 to TargetFrame.",
            "生成開始Frameからどれくらいフレームずれしているかを2F単位で表示します。" => {
                "Shows the frame drift from the generation start frame in 2-frame increments."
            }
            "同じ生成開始Frameにたどり着けるタイムラインへ移動できるFrame1が、どれくらい連続しているか、その下限を設定します。" => {
                "Sets the minimum consecutive range of Frame1 values that can reach the same generation start frame."
            }
            "同じ生成開始Frameへ到達するA入力TargetFrameが、どれくらい連続しているかの下限を設定します。" => {
                "Sets the minimum consecutive range of A-input TargetFrames that reach the same generation start frame."
            }
            "実際にTimeleapしたFrameです。" => "The frame at which the Timeleap actually occurred.",
            "Aボタンを入力するFrameです。" => "The frame on which to press A.",
            "実際に個体生成が始まるFrameです。" => "The frame at which Pokémon generation begins.",
            "生成開始Frameにたどり着けるFrame1候補が、前後にどれくらい連続しているかを2F単位で表示します。" => {
                "Shows, in 2-frame increments, the consecutive range of Frame1 candidates that reach the generation start frame."
            }
            "Frame1準備中…" => "Preparing Frame1...",
            "Frame1計算中…" => "Calculating Frame1...",
            "瞬き判定中…" => "Matching blink timing...",
            "キャンセル" => "Cancel",
            "リトライ" => "Retry",
            "ずれ確認" => "Check drift",
            "瞬き状態を再確認" => "Recheck blink timing",
            "再検索範囲（Frame1） ±" => "Frame1 re-search range ±",
            "予測現在地 ±" => "Predicted current position ±",
            "検索中…" => "Searching...",
            "検索結果なし" => "No results",
            "ポケモン" => "Pokémon",
            "保証V数" => "Guaranteed IV count",
            "卵" => "Egg",
            "自ID" => "Your ID",
            "PIDタイプ" => "PID type",
            "通常色" => "Non-shiny",
            "色違い" => "Shiny",
            "指定" => "Specified",
            "乱数" => "Random",
            "特性固定" => "Lock ability",
            "性格固定" => "Lock nature",
            "性別固定" => "Lock gender",
            "その他" => "Other",
            "メニュー閉じ時の主人公" => "Player character for menu close",
            "男の子" => "Boy",
            "女の子" => "Girl",
            "ポケモン選択でメニュー閉じが自動選択されたときに使用します。" => {
                "Used when menu close is selected automatically for the chosen Pokémon."
            }
            "HP" => "HP",
            "攻撃" => "Attack",
            "防御" => "Defense",
            "特攻" => "Sp. Atk",
            "特防" => "Sp. Def",
            "素早さ" => "Speed",
            "Import" => "Import",
            "読込失敗" => "Import failed",
            "ずれ補正" => "Drift correction",
            "TargetFrame検索を完了できませんでした。" => "TargetFrame search could not be completed.",
            "到達可否計算を完了できませんでした。" => "Reachability check could not be completed.",
            "入力したFrame1には通常Timelineで到達できません。" => {
                "The entered Frame1 is not reachable on the normal timeline."
            }
            "指定した生成開始Frameには到達できません。" => {
                "The specified generation start frame is unreachable."
            }
            "指定したFrame1には到達できません。" => "The specified Frame1 is unreachable.",
            "条件に一致する候補はありません。" => "No candidates match the conditions.",
            "確定したTimelineから候補を計算しています…" => {
                "Calculating candidates from the confirmed timeline..."
            }
            "候補が見つかりません" => "No candidates found",
            "オフセットFrameは整数で入力してください。" => "Offset frame must be an integer.",
            "本番中はオフセットを反映できません。" => {
                "Offsets cannot be applied while the timer is running."
            }
            "タイマー停止中のみ反映できます。" => {
                "This can only be applied while the timer is stopped."
            }
            "キャッシュフォルダにアクセスできません" => "Cannot access the cache folder",
            "SFMTキャッシュを利用できないため、処理を中断しました。" => {
                "The operation was stopped because the SFMT cache is unavailable."
            }
            "キャッシュフォルダへのアクセス権や空き容量を確認してください。" => {
                "Check cache-folder permissions and available disk space."
            }
            "閉じる" => "Close",
            _ => japanese,
        }
    }

    pub fn species_name(self, species_id: u16) -> Option<&'static str> {
        let names = match self.language {
            Language::Japanese => &*SPECIES_NAMES_JA,
            Language::English => &*SPECIES_NAMES_EN,
        };
        names.get(species_id as usize).copied()
    }

    pub fn species_count(self) -> usize {
        match self.language {
            Language::Japanese => SPECIES_NAMES_JA.len(),
            Language::English => SPECIES_NAMES_EN.len(),
        }
    }

    pub fn nature_name(self, nature_id: u8) -> Option<&'static str> {
        let names = match self.language {
            Language::Japanese => &*NATURE_NAMES_JA,
            Language::English => &*NATURE_NAMES_EN,
        };
        names.get(nature_id as usize).copied()
    }

    pub fn timeleap_method(
        self,
        method: crate::application::field::TimeleapMethodSelection,
    ) -> &'static str {
        match method {
            crate::application::field::TimeleapMethodSelection::Dialogue => {
                self.ui_label("会話送り")
            }
            crate::application::field::TimeleapMethodSelection::MenuBoy => {
                self.ui_label("メニュー閉じ（男の子）")
            }
            crate::application::field::TimeleapMethodSelection::MenuGirl => {
                self.ui_label("メニュー閉じ（女の子）")
            }
            crate::application::field::TimeleapMethodSelection::Courier => self.ui_label("配達員"),
        }
    }

    pub fn offset_set_label(self, value: Option<i64>) -> String {
        match (self.language, value) {
            (Language::Japanese, Some(value)) => format!("Offsetを{value}にセット"),
            (Language::English, Some(value)) => format!("Set offset to {value}"),
            (Language::Japanese, None) => "Offsetをセット（計算不可）".to_owned(),
            (Language::English, None) => "Set offset (unavailable)".to_owned(),
        }
    }

    pub fn wc7_import_failed(self) -> &'static str {
        match self.language {
            Language::Japanese => "正常に読み込めませんでした。",
            Language::English => "The file could not be loaded.",
        }
    }

    pub fn fixed_rng_preset_name(
        self,
        preset: &crate::domain::fixed_rng_catalog::FixedRngPreset,
    ) -> String {
        let species = self.species_name(preset.species_id).unwrap_or("-");
        let suffix = if self.language == Language::English {
            match preset.suffix {
                "（波乗り）" => " (Surfing)",
                "（サトピカ）" => " (Ash's Pikachu)",
                "（10%）" => " (10%)",
                "（50%）" => " (50%)",
                "（アローラのすがた）" => " (Alolan)",
                other => other,
            }
        } else {
            preset.suffix
        };
        format!("{species}{suffix}")
    }

    pub fn seconds(self, value: f64) -> String {
        match self.language {
            Language::Japanese => format!("{value:.2} 秒"),
            Language::English => format!("{value:.2} s"),
        }
    }

    pub fn next_seconds(self, value: f64) -> String {
        format!("next >> {value:.2}s")
    }

    pub fn next_seconds_optional(self, value: Option<f64>) -> String {
        value
            .map(|value| self.next_seconds(value))
            .unwrap_or_else(|| "next >> —".into())
    }

    pub fn next_circle_seconds(self, value: f64) -> String {
        let seconds = value.max(0.0).floor();
        match self.language {
            Language::Japanese => format!("○まで{seconds:.0}秒"),
            Language::English => format!("Next ○ in {seconds:.0}s"),
        }
    }

    pub fn production_wait_seconds(self, value: f64) -> String {
        let seconds = value.max(0.0).floor();
        match self.language {
            Language::Japanese => format!("本番待機：{seconds:.0}秒"),
            Language::English => format!("Target wait: {seconds:.0}s"),
        }
    }

    pub fn production_wait_unknown(self) -> &'static str {
        match self.language {
            Language::Japanese => "本番待機：—秒",
            Language::English => "Target wait: —s",
        }
    }

    pub fn next_circle_calculating(self) -> &'static str {
        match self.language {
            Language::Japanese => "○まで計算中…",
            Language::English => "Next ○: calculating...",
        }
    }

    pub fn production_wait_calculating(self) -> &'static str {
        match self.language {
            Language::Japanese => "本番待機：計算中…",
            Language::English => "Target wait: calculating...",
        }
    }

    pub fn next_calculating(self) -> String {
        match self.language {
            Language::Japanese => "next >> 計算中……".into(),
            Language::English => "next >> calculating...".into(),
        }
    }

    text_method!(next_blink_timer, "次の瞬きまで", "Next blink");
    text_method!(target_timer, "Targetまで", "Target timer");
    text_method!(target_passed, "通り過ぎています", "Target passed");
    text_method!(target_calculating, "計算中…", "Calculating...");

    pub fn target_seconds(self, value: f64) -> String {
        match self.language {
            Language::Japanese => format!("Targetまで {:.0}秒", value),
            Language::English => format!("Target in {:.0}s", value),
        }
    }

    pub fn frame1_seconds(self, value: f64) -> String {
        match self.language {
            Language::Japanese => format!("Frame1まで {:.0}秒", value),
            Language::English => format!("Frame1 in {:.0}s", value),
        }
    }

    pub fn next_blink_seconds(self, value: f64) -> String {
        match self.language {
            Language::Japanese => format!("次の瞬きまで {:.0}秒", value),
            Language::English => format!("Next blink in {:.0}s", value),
        }
    }

    pub fn current_frame(self, frame: i64) -> String {
        format!("Current: {frame}")
    }

    pub fn current_frame_optional(self, frame: Option<i64>) -> String {
        frame
            .map(|frame| self.current_frame(frame))
            .unwrap_or_else(|| "Current: —".into())
    }

    pub fn starting_frame(self, frame: i64) -> String {
        // 外部ツールが使う項目名と一致させ、日本語UIでも値をコピーしやすくする。
        format!("StartingFrame : {frame}")
    }

    /// Label used for the B-input origin shown beside the Rotom controls.
    pub fn starting_frame_label(self) -> &'static str {
        match self.language {
            Language::Japanese => "開始位置",
            Language::English => "Starting frame",
        }
    }

    /// Label for the Rotom chatter prediction in correction dialogs.
    pub fn rotom_chatter_label(self) -> &'static str {
        match self.language {
            Language::Japanese => "ロトムのおしゃべり",
            Language::English => "Rotom chatter",
        }
    }

    /// Fastest-close mode uses a separate label from the generic offset setting.
    pub fn fastest_close(self) -> &'static str {
        match self.language {
            Language::Japanese => "最速閉じ",
            Language::English => "Fastest close",
        }
    }

    /// Translate notices at the presentation boundary.
    pub fn notice(self, value: &str) -> String {
        if self.language == Language::Japanese {
            return value.to_owned();
        }
        if let Some(length) = value.strip_prefix("WC7のサイズが不正です:") {
            return format!("Invalid WC7 file size:{length}");
        }
        match value {
            "候補が見つかりません" => "No candidates found".into(),
            "本番中はオフセットを反映できません。" => {
                "Offsets cannot be applied while the timer is running.".into()
            }
            "タイマー停止中のみ反映できます。" => {
                "This can only be applied while the timer is stopped.".into()
            }
            "入力したFrame1には通常Timelineで到達できません。" => {
                "The entered Frame1 is not reachable on the normal timeline.".into()
            }
            "指定したFrame1には到達できません。" => {
                "The specified Frame1 is unreachable.".into()
            }
            "指定した生成開始Frameには到達できません。" => {
                "The specified generation start frame is unreachable.".into()
            }
            "TargetFrame検索を完了できませんでした。" => {
                "TargetFrame search could not be completed.".into()
            }
            "到達可否計算を完了できませんでした。" => {
                "Reachability check could not be completed.".into()
            }
            "SFMTキャッシュを利用できないため、処理を中断しました。" => {
                "The operation was stopped because the SFMT cache is unavailable.".into()
            }
            "キャッシュフォルダへのアクセス権や空き容量を確認してください。" => {
                "Check cache-folder permissions and available disk space.".into()
            }
            "WC7のサイズが不正です:" => "Invalid WC7 file size:".into(),
            "オフセットFrameは整数で入力してください。" => {
                "Offset frame must be an integer.".into()
            }
            _ if !value.chars().any(is_japanese_character) => value.to_owned(),
            _ => "The operation could not be completed. Check the input and try again.".into(),
        }
    }

    pub fn app_error(self, error: &crate::errors::AppError) -> String {
        use crate::errors::{AppError as E, InputField, StateIssue};

        if self.language == Language::Japanese {
            return error.to_string();
        }
        let input_label = |field| match field {
            InputField::SearchStart => "Search start frame",
            InputField::SearchEnd => "Search end frame",
            InputField::TargetFrame => "Target frame",
            InputField::ActualFrame => "Actual frame",
            InputField::NpcCount => "NPC count",
            InputField::NpcNumber => "NPC number",
            InputField::Tolerance => "Blink interval tolerance",
            InputField::WaitFrame => "Wait frame",
            InputField::TargetConsumption => "Target consumption",
            InputField::Offset => "Offset",
            InputField::InitialBranchSearchSpan => "Initial branch search span",
        };
        match error {
            E::Cancelled => "Search canceled.".into(),
            E::InvalidSeed => "Enter the initial SFMT seed in hexadecimal.".into(),
            E::InvalidInteger(field) => format!("{} must be an integer.", input_label(*field)),
            E::NegativeValue(field) => format!("{} must be 0 or greater.", input_label(*field)),
            E::OutOfRange { field, min, max } => {
                format!("{} must be between {min} and {max}.", input_label(*field))
            }
            E::InvalidNpcRange => "Enter the NPC range in a format such as 10~20.".into(),
            E::NpcCountTooLarge => "NPC count is too large.".into(),
            E::NpcNumberTooLarge => "NPC number is too large.".into(),
            E::NpcRangeDoesNotContainTarget => {
                "The NPC range does not include the observed NPC number.".into()
            }
            E::MissingObservation => "Enter at least one blink interval.".into(),
            E::InvalidObservation => "Blink intervals must be non-negative integers.".into(),
            E::TooManyObservations { max } => {
                format!("You can enter up to {max} blink observations.")
            }
            E::InvalidFps => "FPS must be a positive number.".into(),
            E::InvalidOffset { index } => format!("Offset {} must be an integer.", index + 1),
            E::NegativeOffset { index } => {
                format!("Offset {} must be 0 or greater.", index + 1)
            }
            E::OffsetMustBeEven => "Offsets must be entered in 2-frame increments.".into(),
            E::ArithmeticOverflow(operation) => {
                let operation = match operation {
                    crate::errors::ArithmeticOperation::CacheRange => "Cache range",
                    crate::errors::ArithmeticOperation::CacheLength => "Cache length",
                    crate::errors::ArithmeticOperation::TimelinePosition => "Timeline position",
                    crate::errors::ArithmeticOperation::TimelineLength => "Timeline length",
                    crate::errors::ArithmeticOperation::FrameSum => "Frame sum",
                    crate::errors::ArithmeticOperation::OffsetCalculation => "Offset calculation",
                };
                format!("{operation} is too large.")
            }
            E::Cache(_) => "The SFMT cache could not be read or written.".into(),
            E::InvalidState(issue) => match issue {
                StateIssue::CacheUnavailable => "The SFMT cache is unavailable.".into(),
                StateIssue::TimelineUnavailable => "The timeline is unavailable.".into(),
                StateIssue::TimelineCursorUnavailable => {
                    "The timeline position is unavailable.".into()
                }
                StateIssue::TargetUnreachable => "The target frame is unreachable.".into(),
                StateIssue::CandidateUnavailable => "No usable candidate is available.".into(),
                StateIssue::CalculationUnavailable => {
                    "The calculation could not be completed.".into()
                }
                StateIssue::CorrectionModeLocksTimer => {
                    "The timer is locked during correction.".into()
                }
                StateIssue::CandidatesNotUnique => {
                    "The candidates have not been narrowed to one timeline.".into()
                }
                StateIssue::BlinkQueuePending => {
                    "Blink observations are still being processed.".into()
                }
                StateIssue::BlinkTimerCannotContinue => "The blink timer cannot continue.".into(),
                StateIssue::TargetTimerUnavailable => "The target timer is unavailable.".into(),
                StateIssue::ObservationAnchorMissing => {
                    "The observation start position is unavailable.".into()
                }
                StateIssue::TimerTargetPassed => "The target frame has already passed.".into(),
                StateIssue::TimelineLimitReached => "The timeline limit has been reached.".into(),
            },
            E::Persistence(_) => "The application settings could not be saved or loaded.".into(),
            E::CacheWorkerJoin => "The SFMT worker pool failed.".into(),
            E::WorkerPanicked => "A background calculation failed.".into(),
            E::InvalidFieldConfiguration => {
                "Check the timeline input range and observed NPC.".into()
            }
            E::SearchSpanTooWide { max } => {
                format!("The search span must not exceed {max} frames.")
            }
        }
    }

    text_method!(next_circle_count, "次の○まであと", "Blinks to next ○");
    text_method!(
        target_unreachable,
        "このTargetFrameには絶対にたどり着けません",
        "Target frame is unreachable"
    );
    text_method!(
        search_out_of_range,
        "検索の範囲外です",
        "Outside search range"
    );

    text_method!(timeline_sfmt_frame, "Frame", "Frame");
    text_method!(
        timeline_next_blink_seconds,
        "次の瞬きまで（秒）",
        "Next blink (s)"
    );
    text_method!(
        timeline_target_blink_count,
        "Targetまでの瞬き数",
        "Blinks to target"
    );
    text_method!(
        timeline_frame1_blink_count,
        "Frame1までの瞬き数",
        "Blinks to Frame1"
    );

    pub fn target_result(self, exact: bool) -> String {
        format!("Target: {}", if exact { "○" } else { "×" })
    }

    pub fn total_summary(self, frames: i64, _seconds: f64, _fps: f64) -> String {
        match self.language {
            Language::Japanese => format!("合計 {frames}F"),
            Language::English => format!("Total {frames}F"),
        }
    }

    text_method!(hatch_tab, "孵化", "Hatch");
    text_method!(field_tab, "フィールド", "Field");
    text_method!(normal_timer_tab, "タイマー", "Timer");
    text_method!(settings_tab, "設定", "Settings");
    text_method!(b_input, "B入力", "Press B");
    text_method!(
        correction_confirmation,
        "ずれ補正の確認",
        "Confirm drift correction"
    );
    text_method!(
        correction_calculated,
        "ずれを計算しました",
        "Drift correction calculated"
    );
    text_method!(idx_confirmation, "NPC番号の確認", "Confirm NPC number");
    text_method!(
        idx_calculated,
        "NPC番号を計算しました",
        "NPC number inferred"
    );
    text_method!(
        idx_select_question,
        "設定するNPC番号を選択してください。",
        "Select an NPC number to set."
    );
    pub fn idx_set(self, idx: usize) -> String {
        match self.language {
            Language::Japanese => format!("{idx}に設定"),
            Language::English => format!("Set NPC {idx}"),
        }
    }
    pub fn idx_apply_question(self, idx: usize) -> String {
        match self.language {
            Language::Japanese => format!("このNPC番号（{idx}）を入力します。よろしいですか？"),
            Language::English => format!("Set NPC number to {idx}?"),
        }
    }
    pub fn correction_apply_question(self, value: i64) -> String {
        match self.language {
            Language::Japanese => format!("オフセットに{value}Fをセットします。よろしいですか？"),
            Language::English => format!("Set the offset to {value}F?"),
        }
    }

    pub fn correction_rotom_change(self, current: u64, suggested: u64) -> String {
        match self.language {
            Language::Japanese => format!(
                "ロトムのお喋りの確率を{}%から{}%へ変更します。",
                current, suggested
            ),
            Language::English => format!(
                "Change Rotom chatter probability from {}% to {}%.",
                current, suggested
            ),
        }
    }

    pub fn correction_rotom_detail(
        self,
        predicted_talk: bool,
        observed_talk: bool,
        roll: u8,
    ) -> String {
        let predicted = if predicted_talk {
            self.talk_yes()
        } else {
            self.talk_no()
        };
        let observed = if observed_talk {
            self.talk_yes()
        } else {
            self.talk_no()
        };
        match self.language {
            Language::Japanese => {
                format!("想定: {predicted} / 実測相当: {observed}（乱数値%100 = {roll}）")
            }
            Language::English => {
                format!("Expected: {predicted} / Observed: {observed} (RNG value % 100 = {roll})")
            }
        }
    }

    text_method!(yes, "はい", "Yes");
    text_method!(no, "閉じる", "Close");
    text_method!(copy, "コピー", "Copy");
    text_method!(offset, "オフセット", "Offset");
    text_method!(offset_frame, "オフセットFrame", "Offset (Frame)");
    text_method!(memo, "メモ", "Memo");
    text_method!(update, "更新", "Update");
    text_method!(add, "追加", "Add");
    text_method!(stop_editing, "編集をやめる", "Stop editing");
    text_method!(no_memo, "（メモなし）", "(No memo)");
    text_method!(apply, "反映", "Apply");
    text_method!(edit, "編集", "Edit");
    text_method!(delete, "削除", "Delete");
    text_method!(
        hatch_offset_manager,
        "孵化時瞬きのオフセット管理",
        "Hatch Blink Offsets"
    );
    text_method!(
        field_offset_manager,
        "フィールドのオフセット管理",
        "Field Offsets"
    );
    text_method!(
        normal_offset_manager,
        "通常タイマーのオフセット管理",
        "Normal Timer Offsets"
    );
    text_method!(observation, "瞬き観測", "Blink observation");
    text_method!(
        observation_intervals_frames,
        "瞬き間隔（F）",
        "Blink intervals (F)"
    );
    text_method!(range, "範囲:", "Range:");
    text_method!(range_separator, "～", "to");
    text_method!(
        tolerance,
        "観測間隔の許容差（F）:",
        "Blink interval tolerance (F):"
    );
    text_method!(
        cancel_observation,
        "観測キャンセル（Space）",
        "Cancel observation (Space)"
    );
    text_method!(reset_observation, "リセット", "Reset");
    text_method!(
        start_observation,
        "瞬き確認スタート（Space）",
        "Start observation (Space)"
    );
    text_method!(record_blink, "瞬きを入力 (Shift)", "Record blink (Shift)");
    text_method!(conditions, "条件設定", "Conditions");
    text_method!(initial_seed, "初期SEED", "Initial SEED");
    text_method!(target_frame, "Target Frame", "Target Frame");
    text_method!(model_count, "NPC数", "NPC count");
    text_method!(model_number, "NPC番号", "NPC number");
    text_method!(multiple_search, "複数検索", "Search NPC range");
    text_method!(observed_target, "観測対象:", "Observed NPC:");
    text_method!(idx_specified, "NPC番号指定", "Specify NPC number");
    text_method!(unknown, "分からない", "Unknown");
    text_method!(actual_frame, "実際の到達Frame", "Actual reached Frame");
    text_method!(infer_idx, "ずれ補正", "Drift correction");
    text_method!(
        cancel_idx_inference,
        "ずれ補正をキャンセル",
        "Cancel drift correction"
    );
    pub fn idx_fallback(self, requested: &str, candidates: &[usize]) -> String {
        let list = candidates
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        match self.language {
            Language::Japanese => format!("NPC番号 {requested} → {list}？"),
            Language::English => format!("NPC number {requested} → {list}?"),
        }
    }
    text_method!(timeline, "瞬きタイムライン", "Blink timeline");
    text_method!(
        timeline_follow_current,
        "現在位置を固定",
        "Follow current position"
    );
    text_method!(
        auto_tolerance,
        "観測間隔の許容差（F）:",
        "Blink interval tolerance (F):"
    );
    text_method!(target_minus, "Target -", "Target -");
    text_method!(start_plus, "start +", "start +");
    text_method!(set, "Set", "Set");
    text_method!(error_cancel, "候補が見つかりません", "No candidates found");
    text_method!(
        rotom_probability,
        "ロトムのお喋りを考慮する",
        "Consider Rotom chatter"
    );
    text_method!(
        pre_rotom_consumption,
        "ロトム計算前に消費を入れる",
        "Pre-Rotom consumption"
    );
    text_method!(
        initial_branch_scan_span,
        "StartingFrame検索範囲",
        "StartingFrame search range"
    );
    text_method!(probability, "確率", "Probability");
    text_method!(
        npc_initial_load,
        "NPC初期読み込みを考慮する",
        "Account for initial NPC load"
    );
    text_method!(actual_reached, "実際の到達Frame", "Actual reached Frame");
    text_method!(talk, "お喋り:", "Rotom chatter:");
    text_method!(talk_yes, "お喋りあり", "Chatter");
    text_method!(talk_no, "お喋りなし", "No chatter");
    text_method!(blink_correction, "瞬き補正", "Blink correction");
    text_method!(correction, "ずれ補正", "Correction");
    text_method!(
        timeline_reachability,
        "Timeline到達可否",
        "Timeline reachability"
    );
    pub fn target_actual_reachability(
        self,
        target_reachable: bool,
        actual_reachable: bool,
    ) -> String {
        let target = if target_reachable { "○" } else { "×" };
        let actual = if actual_reachable { "○" } else { "×" };
        match self.language {
            Language::Japanese => format!("Target{target}  実測{actual}"),
            Language::English => format!("Target {target}  Actual {actual}"),
        }
    }
    text_method!(
        production_start,
        "本番へ移行 (Enter)",
        "Start target timer (Enter)"
    );
    text_method!(
        correction_start,
        "ずれ検証に進む (Enter)",
        "Continue to drift check (Enter)"
    );
    text_method!(settings, "設定", "Settings");
    text_method!(fps, "FPS", "FPS");
    text_method!(beep_interval, "beep間隔（秒）", "Beep interval (s)");
    text_method!(beep_count, "beep回数", "Beep count");
    text_method!(beep_enabled, "beep音", "Beep sound");
    text_method!(keyboard_enabled, "キーボード操作", "Keyboard controls");
    text_method!(
        candidate_search_sound,
        "候補検索結果音（確定／候補なし）",
        "Candidate result sound (found / none)"
    );
    text_method!(flash_mode, "beep枠の表示", "Beep highlight");
    text_method!(color_scheme, "配色", "Color scheme");
    text_method!(
        wheel_offset,
        "マウスホイールで数値変更",
        "Adjust numeric inputs with mouse wheel"
    );
    text_method!(actual_wait, "実際の待機（Frame）:", "Actual wait (Frame):");
    text_method!(offsets, "オフセット（Frame）:", "Offsets (Frame):");
    text_method!(start_space, "開始（Space）", "Start (Space)");
    text_method!(cancel_space, "キャンセル（Space）", "Cancel (Space)");
    text_method!(language, "言語", "Language");
    text_method!(game_edition, "ゲーム版", "Game");
}

#[cfg(test)]
mod tests {
    use super::{Language, Texts};

    #[test]
    fn wondercard_import_failure_uses_one_localized_generic_message() {
        let japanese = Texts::new(Language::Japanese);
        let english = Texts::new(Language::English);

        assert_eq!(japanese.wc7_import_failed(), "正常に読み込めませんでした。");
        assert_eq!(english.wc7_import_failed(), "The file could not be loaded.");
    }

    #[test]
    fn fixed_rng_categories_follow_the_selected_ui_language() {
        let japanese = Texts::new(Language::Japanese);
        let english = Texts::new(Language::English);

        assert_eq!(
            japanese.fixed_rng_category("Alola Legendary"),
            "アローラの伝説"
        );
        assert_eq!(japanese.fixed_rng_category("配達員"), "配達員");
        assert_eq!(
            english.fixed_rng_category("Alola Legendary"),
            "Alola Legendary"
        );
        assert_eq!(english.fixed_rng_category("配達員"), "Deliveryman");
        for category in crate::domain::fixed_rng_catalog::CATEGORIES
            .iter()
            .chain(crate::domain::fixed_rng_catalog::SM_CATEGORIES.iter())
        {
            if category.name != "-" {
                assert_ne!(
                    japanese.fixed_rng_category(category.name),
                    category.name,
                    "missing Japanese label for {}",
                    category.name
                );
            }
        }
    }

    #[test]
    fn localized_game_data_preserves_stable_ids_and_expected_gen7_entries() {
        let japanese = Texts::new(Language::Japanese);
        let english = Texts::new(Language::English);

        assert_eq!(japanese.species_count(), 808);
        assert_eq!(english.species_count(), 808);
        assert_eq!(japanese.species_name(0), Some("タマゴ"));
        assert_eq!(english.species_name(0), Some("Egg"));
        assert_eq!(japanese.species_name(25), Some("ピカチュウ"));
        assert_eq!(english.species_name(25), Some("Pikachu"));
        assert_eq!(japanese.species_name(807), Some("ゼラオラ"));
        assert_eq!(english.species_name(807), Some("Zeraora"));
        assert_eq!(japanese.species_name(808), None);
        assert_eq!(english.species_name(808), None);

        assert_eq!(japanese.nature_name(0), Some("がんばりや"));
        assert_eq!(english.nature_name(0), Some("Hardy"));
        assert_eq!(japanese.nature_name(24), Some("きまぐれ"));
        assert_eq!(english.nature_name(24), Some("Quirky"));
        assert_eq!(japanese.nature_name(25), None);
        assert_eq!(english.nature_name(25), None);
    }

    #[test]
    fn english_ui_catalog_covers_timeleap_labels_and_fallbacks_unknown_notices() {
        let english = Texts::new(Language::English);
        assert_eq!(english.ui_label("Frame1検索"), "Frame1 Search");
        assert_eq!(
            english.ui_label("再検索範囲（Frame1） ±"),
            "Frame1 re-search range ±"
        );
        assert_eq!(
            english.ui_label("予測現在地 ±"),
            "Predicted current position ±"
        );
        assert_eq!(english.ui_label("猶予①"), "Streak 1");
        assert_eq!(english.offset_set_label(Some(-4)), "Set offset to -4");
        assert_eq!(english.offset_set_label(None), "Set offset (unavailable)");
        assert_eq!(
            english.notice("未来に追加された未知のエラーです。"),
            "The operation could not be completed. Check the input and try again."
        );
        assert_eq!(
            english.notice("an English diagnostic"),
            "an English diagnostic"
        );
    }

    #[test]
    fn english_text_keys_used_directly_by_views_have_no_japanese_residue() {
        let english = Texts::new(Language::English);
        let sources = [
            include_str!("field_view.rs"),
            include_str!("field_view/observation.rs"),
            include_str!("field_view/settings.rs"),
            include_str!("field_view/timeleap_controls.rs"),
            include_str!("field_view/timeleap_search/mod.rs"),
            include_str!("app/correction.rs"),
        ];
        let marker = "texts.ui_label(\"";
        for source in sources {
            let mut remaining = source;
            while let Some(start) = remaining.find(marker) {
                remaining = &remaining[start + marker.len()..];
                let end = remaining
                    .find("\")")
                    .expect("ui_label key should be a string literal");
                let key = &remaining[..end];
                let translated = english.ui_label(key);
                assert!(
                    !translated.chars().any(super::is_japanese_character),
                    "untranslated UI key {key:?} -> {translated:?}"
                );
                remaining = &remaining[end + 2..];
            }
        }
    }

    #[test]
    fn fixed_rng_preset_display_names_are_resolved_in_the_ui_language() {
        let preset = crate::domain::fixed_rng_catalog::CATEGORIES
            .iter()
            .find(|category| category.name == "In-Game Gift")
            .unwrap()
            .presets[1];
        assert_eq!(
            Texts::new(Language::Japanese).fixed_rng_preset_name(&preset),
            "ピカチュウ（サトピカ）"
        );
        assert_eq!(
            Texts::new(Language::English).fixed_rng_preset_name(&preset),
            "Pikachu (Ash's Pikachu)"
        );

        let zygarde = crate::domain::fixed_rng_catalog::selected_preset_for(
            crate::domain::fixed_rng_catalog::GameEdition::SunMoon,
            1,
            9,
        )
        .unwrap();
        assert_eq!(
            Texts::new(Language::English).fixed_rng_preset_name(&zygarde),
            "Zygarde (10%)"
        );
        assert_eq!(Texts::new(Language::Japanese).game_edition(), "ゲーム版");
        assert_eq!(Texts::new(Language::English).game_edition(), "Game");
    }
}
