use super::*;

/// 観測位置から通常の本番開始位置を求める。
///
/// ロトム判定を有効にし、無言なら1消費、お喋りなら2消費として扱う。
pub fn encounter_plan(seed: u32, observed_position: i64, offset: i64) -> Option<EncounterPlan> {
    encounter_plan_with_rotom(seed, observed_position, offset, true)
}

/// 観測位置から本番開始位置を求め、ロトム判定の有無を選択する。
pub fn encounter_plan_with_rotom(
    seed: u32,
    observed_position: i64,
    offset: i64,
    consider_rotom: bool,
) -> Option<EncounterPlan> {
    encounter_plan_with_rotom_rule(
        seed,
        observed_position,
        offset,
        consider_rotom,
        79,
        None,
        0,
        1,
    )
}

/// 実機で確認したロトムのお喋り有無を優先して、本番開始位置を求める。
/// 実測値がない場合だけ、SFMT値と閾値から予測する。
#[cfg(test)]
pub fn encounter_plan_with_observed_rotom(
    seed: u32,
    observed_position: i64,
    offset: i64,
    consider_rotom: bool,
    threshold: u64,
    observed_talk: Option<bool>,
) -> Option<EncounterPlan> {
    encounter_plan_with_observed_rotom_and_initial_load(
        seed,
        observed_position,
        offset,
        consider_rotom,
        threshold,
        observed_talk,
        1,
    )
}

/// B位置でロトムを判定した後、実機のNPC初期読み込みを加えて
/// 本番開始位置を計算する。
pub fn encounter_plan_with_observed_rotom_and_initial_load(
    seed: u32,
    observed_position: i64,
    offset: i64,
    consider_rotom: bool,
    threshold: u64,
    observed_talk: Option<bool>,
    initial_load: i64,
) -> Option<EncounterPlan> {
    encounter_plan_with_pre_rotom_consumption(
        seed,
        observed_position,
        offset,
        consider_rotom,
        threshold,
        observed_talk,
        0,
        initial_load,
    )
}

/// B直後に追加消費を行ってからロトム判定とNPC初期読み込みを行う。
#[allow(clippy::too_many_arguments)]
pub fn encounter_plan_with_pre_rotom_consumption(
    seed: u32,
    observed_position: i64,
    offset: i64,
    consider_rotom: bool,
    threshold: u64,
    observed_talk: Option<bool>,
    pre_rotom_consumption: i64,
    initial_load: i64,
) -> Option<EncounterPlan> {
    encounter_plan_with_rotom_rule(
        seed,
        observed_position,
        offset,
        consider_rotom,
        threshold.min(100),
        observed_talk,
        pre_rotom_consumption,
        initial_load,
    )
}

#[allow(clippy::too_many_arguments)]
fn encounter_plan_with_rotom_rule(
    seed: u32,
    observed_position: i64,
    offset: i64,
    consider_rotom: bool,
    threshold: u64,
    observed_talk: Option<bool>,
    pre_rotom_consumption: i64,
    initial_load: i64,
) -> Option<EncounterPlan> {
    if observed_position < 0 {
        return None;
    }
    if initial_load < 0 {
        return None;
    }
    if pre_rotom_consumption < 0 {
        return None;
    }
    // 観測位置はBボタン直後である。追加消費を適用した位置でロトムを判定し、
    // 無条件の1消費を含むロトム総消費（1または2）を進める。
    // NPC初期読み込みはロトム消費後に行うため、判定値は読み込み数に依存しない。
    let b_position = observed_position.checked_add(pre_rotom_consumption)?;
    let b_value = sfmt_value_at(seed, b_position)?;
    let rotom_consumption = if consider_rotom {
        let predicted_talk = b_value % 100 < threshold;
        let talk = observed_talk.unwrap_or(predicted_talk);
        1 + i64::from(talk)
    } else {
        0
    };
    let encounter_start = b_position
        .checked_add(rotom_consumption)?
        .checked_add(initial_load)?;
    Some(EncounterPlan {
        observed_position,
        b_position,
        rotom_consumption,
        offset,
        encounter_start,
    })
}

/// B直後の位置のSFMT値からロトムのお喋り判定値（%100）を返す。
pub fn rotom_roll_at(seed: u32, observed_position: i64) -> Option<u8> {
    sfmt_value_at(seed, observed_position).map(|value| (value % 100) as u8)
}

fn sfmt_value_at(seed: u32, position: i64) -> Option<u64> {
    if position < 0 {
        return None;
    }
    let mut sfmt = Sfmt::new(seed);
    let mut value = 0;
    for _ in 0..=position {
        value = sfmt.next_u64();
    }
    Some(value)
}
