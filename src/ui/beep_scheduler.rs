use std::time::Instant;

/// 各タイマー区間のbeep予定を保持するUI補助状態。
///
/// 音声デバイスの操作は`UiApp`が担当し、この型は予定時刻の保持・消化と
/// 区間識別用の開始時刻だけを管理する。
#[derive(Default)]
pub(crate) struct BeepScheduler {
    pub(crate) normal: Vec<Instant>,
    pub(crate) article: Vec<Instant>,
    pub(crate) article_start: Option<Instant>,
    pub(crate) field: Vec<Instant>,
    pub(crate) field_start: Option<Instant>,
}

impl BeepScheduler {
    pub(crate) fn drain_due(schedule: &mut Vec<Instant>, now: Instant) -> usize {
        let mut due = 0;
        schedule.retain(|deadline| {
            if *deadline <= now {
                due += 1;
                false
            } else {
                true
            }
        });
        due
    }
}
