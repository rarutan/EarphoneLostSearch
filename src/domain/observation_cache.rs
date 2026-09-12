use super::*;

#[allow(dead_code)]
pub struct BlinkTableRow {
    /// SFMTの64bit出力位置。
    pub frame: i64,
    /// この位置の乱数から決まる次の瞬きまでの秒数。
    pub duration_seconds: f64,
    /// ロトムのお喋り判定に使う、次の位置の乱数値%100。
    pub rotom_roll: Option<u8>,
    /// 表に表示する64bit乱数値。
    pub random_value: u64,
}

/// 観測検索で共有するSeed単位のSFMT出力キャッシュ。
///
/// 開始位置ごとの`BlinkTimeline`を保存せず、SFMTの64bit列をディスクに1本だけ
/// 保存する。観測照合はこの列を順番に読み、候補位置だけをメモリへ返す。
#[derive(Clone)]
pub struct ObservationCache {
    pub range_start: i64,
    pub range_end: i64,
    pub fps: f64,
    path: PathBuf,
    value_count: u64,
}

#[derive(Clone, Copy, Debug)]
/// 観測照合で得られたSFMT位置と次回瞬きの情報。
pub struct ObservationMatch {
    pub start_position: i64,
    pub sfmt_position: i64,
    pub next_blink_ticks: i64,
}

impl ObservationCache {
    /// 検索対象範囲に含まれるSFMT出力数を返す。
    pub fn len(&self) -> usize {
        self.range_end
            .saturating_sub(self.range_start)
            .saturating_add(1)
            .max(0) as usize
    }

    /// キャッシュファイル上で実際に読み出せる絶対SFMT位置の終端。
    /// `range_end`は検索対象範囲の終端であり、照合用先読み分を含まない。
    pub fn available_end(&self) -> i64 {
        self.value_count.saturating_sub(1).min(i64::MAX as u64) as i64
    }

    pub(crate) fn read_values(&self, start: i64, count: usize) -> Result<Vec<u64>, String> {
        if start < 0 || count == 0 {
            return Ok(Vec::new());
        }
        let start = start as u64;
        let end = start
            .checked_add(count as u64)
            .ok_or_else(|| "SFMTキャッシュの読み込み範囲が大きすぎます。".to_string())?;
        if end > self.value_count {
            return Err("SFMTキャッシュの範囲外を読み込もうとしました。".into());
        }
        let mut file = File::open(&self.path)
            .map_err(|error| format!("SFMTキャッシュを開けません: {error}"))?;
        file.seek(SeekFrom::Start(OBSERVATION_CACHE_HEADER_SIZE + start * 8))
            .map_err(|error| format!("SFMTキャッシュを移動できません: {error}"))?;
        let mut bytes = vec![0_u8; count.saturating_mul(8)];
        file.read_exact(&mut bytes)
            .map_err(|error| format!("SFMTキャッシュを読み込めません: {error}"))?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
            .collect())
    }

    #[allow(dead_code)]
    pub fn table_rows(&self, start: i64, count: usize) -> Result<Vec<BlinkTableRow>, String> {
        let available = self
            .value_count
            .saturating_sub(start.max(0) as u64)
            .min(usize::MAX as u64) as usize;
        let values = self.read_values(start, count.saturating_add(1).min(available))?;
        Ok(values
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, value)| {
                let rotom_roll = values.get(index + 1).map(|next| (next % 100) as u8);
                let interval_ticks = blink_interval_ticks(*value);
                BlinkTableRow {
                    frame: start + index as i64,
                    duration_seconds: blink_ticks_to_seconds(interval_ticks as i64, self.fps),
                    rotom_roll,
                    random_value: *value,
                }
            })
            .collect())
    }

    /// 指定位置の瞬き間隔を1/30秒tickで返す。
    pub fn interval_ticks_at(&self, position: i64) -> Result<Option<i64>, String> {
        let Some(value) = self.read_values(position, 1)?.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(blink_interval_ticks(value) as i64))
    }

    /// 表示Frameへ丸める前の、次の瞬きまでの正確な実時間。
    pub fn interval_seconds_at(&self, position: i64) -> Result<Option<f64>, String> {
        let Some(value) = self.read_values(position, 1)?.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(blink_interval_seconds(value, self.fps)))
    }

    /// 指定区間の瞬き間隔を一度のファイル読み込みで取得する。
    /// 右上案内の猶予計算で1Frameごとにファイルを開き直さないために使う。
    pub fn interval_seconds_range(&self, start: i64, count: usize) -> Result<Vec<f64>, String> {
        Ok(self
            .read_values(start, count)?
            .into_iter()
            .map(|value| blink_interval_seconds(value, self.fps))
            .collect())
    }

    /// `observed`と予測値は、どちらも表示Frame単位で受け取る。
    ///
    /// SFMT値から得られる瞬き間隔は内部では1/30秒tickだが、検索時には
    /// 外部ツール/FieldTimelineと同じく`tick * 2`の表示Frameへ変換する。
    /// 観測間隔列と一致する開始位置を検索する。
    ///
    /// 検索結果はSFMT位置の昇順で返し、許容差は表示Frame単位で適用する。
    pub fn find_matches(
        &self,
        observed: &[i64],
        tolerance: i64,
    ) -> Result<Vec<ObservationMatch>, String> {
        if observed.is_empty() {
            return Ok(Vec::new());
        }
        let required = observed.len().saturating_add(1);
        // MAX_SFMT_FRAMEのキャッシュは共有上限で切られるため、末尾付近では間隔列に
        // 必要な先読み値が不足することがある。I/Oエラーで検索全体を空にせず、読み出せる
        // 先頭部分だけを照合する。
        let last_cached = self.value_count.saturating_sub(1).min(i64::MAX as u64) as i64;
        let latest_start = self
            .range_end
            .min(last_cached.saturating_sub(observed.len() as i64));
        if latest_start < self.range_start {
            return Ok(Vec::new());
        }
        let end = latest_start
            .checked_add(observed.len() as i64)
            .ok_or_else(|| "観測照合範囲が大きすぎます。".to_string())?;
        let count = end
            .checked_sub(self.range_start)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| "観測照合範囲が不正です。".to_string())? as usize;
        let mut reader = BufReader::new(
            File::open(&self.path)
                .map_err(|error| format!("SFMTキャッシュを開けません: {error}"))?,
        );
        reader
            .seek(SeekFrom::Start(
                OBSERVATION_CACHE_HEADER_SIZE + self.range_start as u64 * 8,
            ))
            .map_err(|error| format!("SFMTキャッシュを移動できません: {error}"))?;
        let mut intervals = VecDeque::with_capacity(required);
        let mut matches = Vec::new();
        let mut bytes = vec![0_u8; 64 * 1024 * 8];
        let mut processed = 0_usize;
        while processed < count {
            let read_count = (count - processed).min(64 * 1024);
            reader
                .read_exact(&mut bytes[..read_count * 8])
                .map_err(|error| format!("SFMTキャッシュを読み込めません: {error}"))?;
            for chunk in bytes[..read_count * 8].chunks_exact(8) {
                let value = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
                let value_index = processed;
                let interval_ticks = blink_interval_ticks(value) as i64;
                let interval_frames = interval_ticks * GAME_FRAMES_PER_BLINK_TICK;
                intervals.push_back(interval_frames);
                if intervals.len() >= required {
                    let start = self.range_start + value_index as i64 - observed.len() as i64;
                    let matched = observed.iter().enumerate().all(|(offset, expected)| {
                        (intervals[offset] - expected).abs() <= tolerance
                    });
                    if matched {
                        matches.push(ObservationMatch {
                            start_position: start,
                            sfmt_position: start + observed.len() as i64,
                            next_blink_ticks: intervals[observed.len()]
                                / GAME_FRAMES_PER_BLINK_TICK,
                        });
                    }
                    intervals.pop_front();
                }
                processed += 1;
            }
        }
        Ok(matches)
    }
}

pub(crate) fn blink_interval_ticks(value: u64) -> u64 {
    value % BLINK_INTERVAL_MODULUS + BLINK_INTERVAL_BASE_TICKS
}

pub(crate) fn blink_interval_seconds(value: u64, fps: f64) -> f64 {
    blink_ticks_to_seconds(blink_interval_ticks(value) as i64, fps)
}

/// SFMT列をディスクキャッシュへ生成する。キャッシュはSeedと終点が同じなら再利用する。
pub fn generate_observation_cache(
    seed: u32,
    range: RangeInclusive<i64>,
    _target: i64,
    fps: f64,
) -> Result<ObservationCache, String> {
    generate_observation_cache_cancelable(seed, range, _target, fps, None)
}

pub(crate) fn generate_observation_cache_cancelable(
    seed: u32,
    range: RangeInclusive<i64>,
    _target: i64,
    fps: f64,
    cancel: Option<&AtomicBool>,
) -> Result<ObservationCache, String> {
    let range_start = (*range.start()).max(0);
    let range_end = (*range.end()).max(range_start);
    // Targetは本番到達可否の入力であり、観測用SFMT列の長さを決めない。
    if range_end > MAX_SFMT_FRAME {
        return Err(format!(
            "SFMTキャッシュの終点は{}以下で入力してください。",
            MAX_SFMT_FRAME
        ));
    }
    let last_position = range_end
        .saturating_add(OBSERVATION_LOOKAHEAD)
        .min(MAX_SFMT_FRAME);
    let value_count = last_position
        .checked_add(1)
        .ok_or_else(|| "SFMTキャッシュの範囲が大きすぎます。".to_string())?
        as u64;

    let store = crate::infrastructure::cache_store::CacheStore::default();
    store.ensure_directory()?;
    let path = store.path(seed, last_position);
    if store.is_valid(
        &path,
        seed,
        value_count,
        OBSERVATION_CACHE_VERSION,
        OBSERVATION_CACHE_HEADER_SIZE,
    )? {
        return Ok(ObservationCache {
            range_start,
            range_end,
            fps,
            path,
            value_count,
        });
    }

    let temporary_path = store.temporary_path(seed, last_position);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| format!("SFMTキャッシュを作れません: {error}"))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(b"HBSFMT\0\0")
        .and_then(|_| writer.write_all(&OBSERVATION_CACHE_VERSION.to_le_bytes()))
        .and_then(|_| writer.write_all(&seed.to_le_bytes()))
        .and_then(|_| writer.write_all(&value_count.to_le_bytes()))
        .map_err(|error| format!("SFMTキャッシュのヘッダーを書けません: {error}"))?;
    let mut sfmt = Sfmt::new(seed);
    let mut bytes = Vec::with_capacity(1024 * 1024);
    for index in 0..value_count {
        if index % 4096 == 0 && cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            drop(writer);
            let _ = fs::remove_file(&temporary_path);
            return Err("検索をキャンセルしました。".into());
        }
        bytes.extend_from_slice(&sfmt.next_u64().to_le_bytes());
        if bytes.len() >= 1024 * 1024 {
            writer
                .write_all(&bytes)
                .map_err(|error| format!("SFMTキャッシュを書き込めません: {error}"))?;
            bytes.clear();
        }
    }
    if !bytes.is_empty() {
        writer
            .write_all(&bytes)
            .map_err(|error| format!("SFMTキャッシュを書き込めません: {error}"))?;
    }
    writer
        .flush()
        .map_err(|error| format!("SFMTキャッシュを確定できません: {error}"))?;
    drop(writer);
    if path.exists()
        && store.is_valid(
            &path,
            seed,
            value_count,
            OBSERVATION_CACHE_VERSION,
            OBSERVATION_CACHE_HEADER_SIZE,
        )?
    {
        let _ = fs::remove_file(&temporary_path);
    } else {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("古いSFMTキャッシュを置き換えられません: {error}"))?;
        }
        fs::rename(&temporary_path, &path)
            .map_err(|error| format!("SFMTキャッシュを配置できません: {error}"))?;
    }
    Ok(ObservationCache {
        range_start,
        range_end,
        fps,
        path,
        value_count,
    })
}
