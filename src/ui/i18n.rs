use serde::{Deserialize, Serialize};

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

    pub fn app_title(self) -> &'static str {
        "EarphoneLostSearch"
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

    pub fn next_calculating(self) -> String {
        match self.language {
            Language::Japanese => "next >> 計算中……".into(),
            Language::English => "next >> calculating...".into(),
        }
    }

    text_method!(next_blink_timer, "次の瞬きまで", "Next blink");
    text_method!(target_timer, "Targetまで", "Target timer");
    text_method!(target_passed, "通り過ぎています", "Target passed");

    pub fn target_seconds(self, value: f64) -> String {
        match self.language {
            Language::Japanese => format!("Targetまで {:.0}秒", value),
            Language::English => format!("Target in {:.0}s", value),
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

    /// Translate status messages at the presentation boundary. Unknown domain
    /// messages are returned unchanged until the validation layer gains
    /// structured error codes.
    pub fn status(self, value: &str) -> String {
        if self.language == Language::Japanese {
            return value.to_owned();
        }
        match value {
            "候補が見つかりません" => "No candidates found".into(),
            "現在位置を一意に特定しました。追加の瞬き入力は停止しています。" => {
                "Current position identified. Further blink input is disabled.".into()
            }
            "タイムラインを計算中です。観測を続けてください。" => {
                "Building the timeline. Continue observing.".into()
            }
            "瞬き間隔を照合中…" => "Matching blink intervals...".into(),
            "SFMT・Timelineを準備中…" => "Preparing SFMT and timeline...".into(),
            "ずれ補正を計算中…" => "Calculating drift correction...".into(),
            "ずれ補正をキャンセル中…" => "Cancelling drift correction...".into(),
            "タイムライン仕様の計算に失敗しました。" => {
                "Timeline calculation failed.".into()
            }
            "SFMTプールの計算に失敗しました。" => {
                "SFMT pool generation failed.".into()
            }
            "ずれ補正に失敗しました。" => "Drift correction failed.".into(),
            "Target Frameに到達しました。終了です。" => {
                "Target frame reached. Done.".into()
            }
            "本番中はオフセットを反映できません。" => {
                "Offsets cannot be applied while the timer is running.".into()
            }
            "タイマー停止中のみ反映できます。" => {
                "This can only be applied while the timer is stopped.".into()
            }
            "観測をキャンセルしました。" => "Observation cancelled.".into(),
            "検索をキャンセルしました。" => "Search cancelled.".into(),
            "初期SEEDを入力してください。" => "Enter an initial seed.".into(),
            "初期SEEDは8桁以内の16進数で入力してください。" => {
                "Initial seed must be at most 8 hexadecimal digits.".into()
            }
            "初期SFMT SEEDは16進数で入力してください。" => {
                "Initial SFMT seed must be hexadecimal.".into()
            }
            "Target Frameは整数で入力してください。" => {
                "Target frame must be an integer.".into()
            }
            "実際に出現したTarget Frameは整数で入力してください。" => {
                "Observed target frame must be an integer.".into()
            }
            "実際に到達したFrameは0以上で入力してください。" => {
                "Observed frame must be zero or greater.".into()
            }
            "候補が一意になるまでタイマーは開始しません。" => {
                "The timer starts after one candidate remains.".into()
            }
            "タイマーを開始する候補を選択してください。" => {
                "Select a candidate before starting the timer.".into()
            }
            "Target消費数に到達できない候補です。" => {
                "This candidate cannot reach the target frame.".into()
            }
            "TargetFrameのタイマーはまだ利用できません。" => {
                "The target-frame timer is not available yet.".into()
            }
            "TargetFrameを通り過ぎています。" => "The target frame has passed.".into(),
            "オフセットは2F単位で入力してください。" => {
                "Offset must be entered in 2-frame increments.".into()
            }
            "オフセットは0以上の表示Fで入力してください。" => {
                "Offset must be zero or greater.".into()
            }
            "実際の待機Frameは整数で入力してください。" => {
                "Actual wait must be an integer.".into()
            }
            "実際の待機Frameは0以上で入力してください。" => {
                "Actual wait must be zero or greater.".into()
            }
            "FPSは0より大きい数値で入力してください。" => {
                "FPS must be greater than zero.".into()
            }
            _ if value.starts_with("候補が見つかりません") => {
                "No candidates found".into()
            }
            _ => value.to_owned(),
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
}
