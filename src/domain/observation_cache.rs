use super::*;
use crate::errors::{AppError, AppResult, ArithmeticOperation, CacheError, CacheIoOperation};

// 8 MiB弱のSFMT出力数。SFMT内部状態が312個の64bit出力ごとに更新されるため、
// チャンク末尾もその境界に揃える。値は64の倍数にもなっている。
pub(crate) const CACHE_EXTENSION_CHUNK_VALUES: u64 = 1_048_320;

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
    pub seed: u32,
    pub range_start: i64,
    pub range_end: i64,
    pub fps: f64,
    cache_file: Arc<crate::infrastructure::cache_store::CacheFileLease>,
    value_count: Arc<std::sync::atomic::AtomicU64>,
    extension_sfmt: Arc<std::sync::Mutex<Sfmt>>,
}

#[derive(Debug)]
pub(crate) enum CacheReadError {
    OutOfRange,
    Io {
        operation: CacheIoOperation,
        source: std::io::Error,
    },
}

impl std::fmt::Display for CacheReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange => formatter.write_str("キャッシュの読み込み範囲外です"),
            Self::Io { source, .. } => write!(formatter, "{source}"),
        }
    }
}

impl std::error::Error for CacheReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::OutOfRange => None,
        }
    }
}

impl From<CacheReadError> for AppError {
    fn from(error: CacheReadError) -> Self {
        match error {
            CacheReadError::OutOfRange => AppError::Cache(CacheError::OutOfRange),
            CacheReadError::Io { operation, source } => {
                AppError::Cache(CacheError::Io { operation, source })
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// 観測照合で得られたSFMT位置と次回瞬きの情報。
pub struct ObservationMatch {
    pub start_position: i64,
    pub sfmt_position: i64,
    pub next_blink_ticks: i64,
}

impl ObservationCache {
    /// キャッシュファイル上で実際に読み出せる絶対SFMT位置の終端。
    /// `range_end`は検索対象範囲の終端であり、照合用先読み分を含まない。
    pub fn available_end(&self) -> i64 {
        self.value_count
            .load(Ordering::Acquire)
            .saturating_sub(1)
            .min(i64::MAX as u64) as i64
    }

    pub(crate) fn verify_readable(&self, position: i64) -> Result<(), CacheReadError> {
        let mut file = File::open(&self.cache_file.path).map_err(|source| CacheReadError::Io {
            operation: CacheIoOperation::Open,
            source,
        })?;
        file.seek(SeekFrom::Start(
            OBSERVATION_CACHE_HEADER_SIZE + position as u64 * 8,
        ))
        .map_err(|source| CacheReadError::Io {
            operation: CacheIoOperation::Seek,
            source,
        })?;
        let mut value = [0_u8; 8];
        file.read_exact(&mut value)
            .map_err(|source| CacheReadError::Io {
                operation: CacheIoOperation::Read,
                source,
            })
    }

    pub(crate) fn read_values(&self, start: i64, count: usize) -> AppResult<Vec<u64>> {
        if start < 0 || count == 0 {
            return Ok(Vec::new());
        }
        let start = start as u64;
        let end = start
            .checked_add(count as u64)
            .ok_or(AppError::ArithmeticOverflow(
                ArithmeticOperation::CacheRange,
            ))?;
        if end > self.value_count.load(Ordering::Acquire) {
            return Err(AppError::Cache(CacheError::OutOfRange));
        }
        let mut file = File::open(&self.cache_file.path).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Open,
                source,
            })
        })?;
        file.seek(SeekFrom::Start(OBSERVATION_CACHE_HEADER_SIZE + start * 8))
            .map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::Seek,
                    source,
                })
            })?;
        let mut bytes = vec![0_u8; count.saturating_mul(8)];
        file.read_exact(&mut bytes).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Read,
                source,
            })
        })?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
            .collect())
    }

    pub(crate) fn sfmt_at_checked(&self, position: i64) -> Result<Option<Sfmt>, CacheReadError> {
        if position < 0 || position > self.available_end() {
            return Ok(None);
        }
        let block_start = position / (N32 as i64 / 2) * (N32 as i64 / 2);
        let count = usize::try_from(self.available_end() - block_start + 1)
            .map_err(|_| CacheReadError::OutOfRange)?
            .min(N32 / 2);
        let mut file = File::open(&self.cache_file.path).map_err(|source| CacheReadError::Io {
            operation: CacheIoOperation::Open,
            source,
        })?;
        file.seek(SeekFrom::Start(
            OBSERVATION_CACHE_HEADER_SIZE + block_start as u64 * 8,
        ))
        .map_err(|source| CacheReadError::Io {
            operation: CacheIoOperation::Seek,
            source,
        })?;
        let mut bytes = vec![0_u8; count.saturating_mul(8)];
        file.read_exact(&mut bytes)
            .map_err(|source| CacheReadError::Io {
                operation: CacheIoOperation::Read,
                source,
            })?;
        let values = bytes
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
            .collect::<Vec<_>>();
        Ok(Sfmt::from_cached_output_block(
            position,
            block_start,
            &values,
        ))
    }

    #[allow(dead_code)]
    pub fn table_rows(&self, start: i64, count: usize) -> AppResult<Vec<BlinkTableRow>> {
        let available = self
            .value_count
            .load(Ordering::Acquire)
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
    pub fn interval_ticks_at(&self, position: i64) -> AppResult<Option<i64>> {
        let Some(value) = self.read_values(position, 1)?.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(blink_interval_ticks(value) as i64))
    }

    /// 表示Frameへ丸める前の、次の瞬きまでの正確な実時間。
    pub fn interval_seconds_at(&self, position: i64) -> AppResult<Option<f64>> {
        let Some(value) = self.read_values(position, 1)?.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(blink_interval_seconds(value, self.fps)))
    }

    /// 指定区間の瞬き間隔を一度のファイル読み込みで取得する。
    /// 右上案内の猶予計算で1Frameごとにファイルを開き直さないために使う。
    pub fn interval_seconds_range(&self, start: i64, count: usize) -> AppResult<Vec<f64>> {
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
    ) -> AppResult<Vec<ObservationMatch>> {
        if observed.is_empty() {
            return Ok(Vec::new());
        }
        let required = observed.len().saturating_add(1);
        // MAX_SFMT_FRAMEのキャッシュは共有上限で切られるため、末尾付近では間隔列に
        // 必要な先読み値が不足することがある。I/Oエラーで検索全体を空にせず、読み出せる
        // 先頭部分だけを照合する。
        let value_count = self.value_count.load(Ordering::Acquire);
        let last_cached = value_count.saturating_sub(1).min(i64::MAX as u64) as i64;
        let latest_start = self
            .range_end
            .min(last_cached.saturating_sub(observed.len() as i64));
        if latest_start < self.range_start {
            return Ok(Vec::new());
        }
        let end =
            latest_start
                .checked_add(observed.len() as i64)
                .ok_or(AppError::ArithmeticOverflow(
                    ArithmeticOperation::CacheRange,
                ))?;
        let count = end
            .checked_sub(self.range_start)
            .and_then(|value| value.checked_add(1))
            .ok_or(AppError::Cache(CacheError::InvalidRange))? as usize;
        let mut reader = BufReader::new(File::open(&self.cache_file.path).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Open,
                source,
            })
        })?);
        reader
            .seek(SeekFrom::Start(
                OBSERVATION_CACHE_HEADER_SIZE + self.range_start as u64 * 8,
            ))
            .map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::Seek,
                    source,
                })
            })?;
        let mut intervals = VecDeque::with_capacity(required);
        let mut matches = Vec::new();
        let mut bytes = vec![0_u8; 64 * 1024 * 8];
        let mut processed = 0_usize;
        while processed < count {
            let read_count = (count - processed).min(64 * 1024);
            reader
                .read_exact(&mut bytes[..read_count * 8])
                .map_err(|source| {
                    AppError::Cache(CacheError::Io {
                        operation: CacheIoOperation::Read,
                        source,
                    })
                })?;
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

    /// 既生成範囲の末尾からSFMTを進め、要求位置までキャッシュをチャンク追記する。
    ///
    /// 呼び出し側はワーカー上で実行する。各チャンクのデータとヘッダーを書き終えて
    /// から公開済み要素数と次のSFMT状態を更新するため、途中でキャンセル・I/O失敗しても
    /// 最後に確定した位置から再開できる。
    pub(crate) fn ensure_through(
        &self,
        position: i64,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<()> {
        if !(0..=MAX_SFMT_FRAME).contains(&position) {
            return Err(AppError::Cache(CacheError::OutOfRange));
        }
        let chunk_width = CACHE_EXTENSION_CHUNK_VALUES.min(i64::MAX as u64) as i64;
        if position <= self.available_end().saturating_sub(chunk_width) {
            return Ok(());
        }

        let mut next_sfmt = self
            .extension_sfmt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let max_count = MAX_SFMT_FRAME as u64 + 1;
        let mut current_count = self.value_count.load(Ordering::Acquire);
        if position <= (current_count.saturating_sub(1) as i64).saturating_sub(chunk_width) {
            return Ok(());
        }
        let required_count = position as u64 + 1;
        // 要求地点の先に1チャンク確保する。要求が現在の末尾から1チャンク以内に
        // 入った時点で次のチャンクを追記し、読み手が末尾へ追いつかないようにする。
        let buffered_count = required_count.saturating_add(CACHE_EXTENSION_CHUNK_VALUES);
        let rounded_count = buffered_count.saturating_add(CACHE_EXTENSION_CHUNK_VALUES - 1)
            / CACHE_EXTENSION_CHUNK_VALUES
            * CACHE_EXTENSION_CHUNK_VALUES;
        let target_count = rounded_count.min(max_count);

        while current_count < target_count {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let chunk_end = current_count
                .saturating_add(CACHE_EXTENSION_CHUNK_VALUES)
                .min(target_count);
            let chunk_count = usize::try_from(chunk_end - current_count)
                .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::CacheRange))?;
            let mut chunk_sfmt = next_sfmt.clone();
            let mut bytes = Vec::with_capacity(chunk_count.saturating_mul(8));
            for index in 0..chunk_count {
                if index % 4096 == 0 && cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                    return Err(AppError::Cancelled);
                }
                bytes.extend_from_slice(&chunk_sfmt.next_u64().to_le_bytes());
            }

            let mut file = OpenOptions::new()
                .write(true)
                .open(&self.cache_file.path)
                .map_err(|source| {
                    AppError::Cache(CacheError::Io {
                        operation: CacheIoOperation::Open,
                        source,
                    })
                })?;
            let valid_length = OBSERVATION_CACHE_HEADER_SIZE
                .checked_add(current_count.saturating_mul(8))
                .ok_or(AppError::ArithmeticOverflow(
                    ArithmeticOperation::CacheLength,
                ))?;
            file.set_len(valid_length).map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::SetLength,
                    source,
                })
            })?;
            file.seek(SeekFrom::Start(valid_length))
                .and_then(|_| file.write_all(&bytes))
                .and_then(|_| file.flush())
                .map_err(|source| {
                    AppError::Cache(CacheError::Io {
                        operation: CacheIoOperation::Write,
                        source,
                    })
                })?;
            file.seek(SeekFrom::Start(16))
                .and_then(|_| file.write_all(&chunk_end.to_le_bytes()))
                .and_then(|_| file.flush())
                .map_err(|source| {
                    AppError::Cache(CacheError::Io {
                        operation: CacheIoOperation::Write,
                        source,
                    })
                })?;

            *next_sfmt = chunk_sfmt;
            current_count = chunk_end;
            self.value_count.store(current_count, Ordering::Release);
        }
        Ok(())
    }
}

pub(crate) fn blink_interval_ticks(value: u64) -> u64 {
    value % BLINK_INTERVAL_MODULUS + BLINK_INTERVAL_BASE_TICKS
}

pub(crate) fn blink_interval_seconds(value: u64, fps: f64) -> f64 {
    blink_ticks_to_seconds(blink_interval_ticks(value) as i64, fps)
}

fn restore_sfmt_after_cached_values(path: &std::path::Path, value_count: u64) -> AppResult<Sfmt> {
    if value_count == 0 {
        return Err(AppError::Cache(CacheError::EmptyCache));
    }
    let block_width = (N32 / 2) as u64;
    let block_start = (value_count - 1) / block_width * block_width;
    let block_count = usize::try_from(value_count - block_start)
        .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::CacheRange))?;
    let mut file = File::open(path).map_err(|source| {
        AppError::Cache(CacheError::Io {
            operation: CacheIoOperation::Open,
            source,
        })
    })?;
    file.seek(SeekFrom::Start(
        OBSERVATION_CACHE_HEADER_SIZE + block_start.saturating_mul(8),
    ))
    .map_err(|source| {
        AppError::Cache(CacheError::Io {
            operation: CacheIoOperation::Seek,
            source,
        })
    })?;
    let mut bytes = vec![0_u8; block_count.saturating_mul(8)];
    file.read_exact(&mut bytes).map_err(|source| {
        AppError::Cache(CacheError::Io {
            operation: CacheIoOperation::Read,
            source,
        })
    })?;
    let values = bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
        .collect::<Vec<_>>();
    let mut sfmt = Sfmt::from_cached_output_block(block_start as i64, block_start as i64, &values)
        .ok_or(AppError::Cache(CacheError::StateUnavailable))?;
    for _ in 0..block_count {
        sfmt.next_u64();
    }
    Ok(sfmt)
}

/// SFMT列をディスクキャッシュへ生成する。キャッシュはSeedと終点が同じなら再利用する。
pub fn generate_observation_cache(
    seed: u32,
    range: RangeInclusive<i64>,
    _target: i64,
    fps: f64,
) -> AppResult<ObservationCache> {
    generate_observation_cache_cancelable(seed, range, _target, fps, None)
}

pub(crate) fn generate_observation_cache_cancelable(
    seed: u32,
    range: RangeInclusive<i64>,
    _target: i64,
    fps: f64,
    cancel: Option<&AtomicBool>,
) -> AppResult<ObservationCache> {
    let range_start = (*range.start()).max(0);
    let range_end = (*range.end()).max(range_start);
    // Targetは本番到達可否の入力であり、観測用SFMT列の長さを決めない。
    if range_end > MAX_SFMT_FRAME {
        return Err(AppError::OutOfRange {
            field: crate::errors::InputField::SearchEnd,
            min: 0,
            max: MAX_SFMT_FRAME,
        });
    }
    let required_last_position = range_end
        .saturating_add(OBSERVATION_LOOKAHEAD)
        .min(MAX_SFMT_FRAME);
    let required_count =
        required_last_position
            .checked_add(1)
            .ok_or(AppError::ArithmeticOverflow(
                ArithmeticOperation::CacheRange,
            ))? as u64;
    // 初回生成も延長と同じ絶対チャンク境界で終え、次回追記の先頭を揃える。
    let buffered_count = required_count.saturating_add(CACHE_EXTENSION_CHUNK_VALUES);
    let value_count = buffered_count.saturating_add(CACHE_EXTENSION_CHUNK_VALUES - 1)
        / CACHE_EXTENSION_CHUNK_VALUES
        * CACHE_EXTENSION_CHUNK_VALUES;
    let value_count = value_count.min(MAX_SFMT_FRAME as u64 + 1);
    let last_position = value_count.saturating_sub(1) as i64;

    let store = crate::infrastructure::cache_store::CacheStore::default();
    store.ensure_directory()?;
    let (path, cache_file) = store.lease_cache_file(seed, last_position);
    if store.is_valid(
        &path,
        seed,
        value_count,
        OBSERVATION_CACHE_VERSION,
        OBSERVATION_CACHE_HEADER_SIZE,
    )? {
        return Ok(ObservationCache {
            seed,
            range_start,
            range_end,
            fps,
            cache_file,
            value_count: Arc::new(std::sync::atomic::AtomicU64::new(value_count)),
            extension_sfmt: Arc::new(std::sync::Mutex::new(restore_sfmt_after_cached_values(
                &path,
                value_count,
            )?)),
        });
    }

    let (temporary_path, _temporary_file, file) =
        store.create_temporary_file(seed, last_position)?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(b"HBSFMT\0\0")
        .and_then(|_| writer.write_all(&OBSERVATION_CACHE_VERSION.to_le_bytes()))
        .and_then(|_| writer.write_all(&seed.to_le_bytes()))
        .and_then(|_| writer.write_all(&value_count.to_le_bytes()))
        .map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Write,
                source,
            })
        })?;
    let mut sfmt = Sfmt::new(seed);
    let mut bytes = Vec::with_capacity(1024 * 1024);
    for index in 0..value_count {
        if index % 4096 == 0 && cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            drop(writer);
            let _ = fs::remove_file(&temporary_path);
            return Err(AppError::Cancelled);
        }
        bytes.extend_from_slice(&sfmt.next_u64().to_le_bytes());
        if bytes.len() >= 1024 * 1024 {
            writer.write_all(&bytes).map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::Write,
                    source,
                })
            })?;
            bytes.clear();
        }
    }
    if !bytes.is_empty() {
        writer.write_all(&bytes).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Write,
                source,
            })
        })?;
    }
    writer.flush().map_err(|source| {
        AppError::Cache(CacheError::Io {
            operation: CacheIoOperation::Commit,
            source,
        })
    })?;
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
            fs::remove_file(&path).map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::Replace,
                    source,
                })
            })?;
        }
        fs::rename(&temporary_path, &path).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Replace,
                source,
            })
        })?;
    }
    Ok(ObservationCache {
        seed,
        range_start,
        range_end,
        fps,
        cache_file,
        value_count: Arc::new(std::sync::atomic::AtomicU64::new(value_count)),
        extension_sfmt: Arc::new(std::sync::Mutex::new(sfmt)),
    })
}

#[cfg(test)]
mod cache_lifecycle_tests {
    use super::*;
    use crate::infrastructure::cache_store::CacheStore;
    use std::path::Path;
    use std::sync::Barrier;
    use std::time::{Duration, Instant};

    const HIGH_START: i64 = 49_500_000;
    const HIGH_END: i64 = HIGH_START + 100;
    const TEST_SEED: u32 = 0x6A31_D5C2;

    fn generate_high_frame_cache() -> ObservationCache {
        generate_observation_cache_cancelable(
            TEST_SEED,
            HIGH_START..=HIGH_END,
            HIGH_END,
            59.8621,
            None,
        )
        .expect("generate high-frame observation cache")
    }

    #[test]
    fn cache_read_io_failures_keep_the_operation_as_a_typed_error() {
        let cache = generate_observation_cache(TEST_SEED, 0..=0, 0, 59.8621)
            .expect("generate observation cache");
        fs::remove_file(&cache.cache_file.path).expect("remove cache file");

        let error = cache.read_values(0, 1).expect_err("missing file must fail");
        assert!(matches!(
            error,
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Open,
                ..
            })
        ));
    }

    #[test]
    fn cache_extension_appends_the_same_sfmt_sequence_and_is_shared_by_clones() {
        let seed = 0xC0FF_EE11;
        let cache = generate_observation_cache(seed, 0..=0, 0, 59.8621).unwrap();
        assert_eq!(CACHE_EXTENSION_CHUNK_VALUES % 64, 0);
        assert_eq!(CACHE_EXTENSION_CHUNK_VALUES % (N32 as u64 / 2), 0);
        assert_eq!(
            cache.available_end() as u64 + 1,
            CACHE_EXTENSION_CHUNK_VALUES * 3
        );
        let initial_end = cache.available_end();
        let refill_position = initial_end - CACHE_EXTENSION_CHUNK_VALUES as i64 + 1;
        cache.ensure_through(refill_position, None).unwrap();
        assert_eq!(
            cache.available_end(),
            initial_end + CACHE_EXTENSION_CHUNK_VALUES as i64
        );
        let clone = cache.clone();
        let old_end = cache.available_end();
        let requested_end = old_end + 3;
        let mut expected = Sfmt::new(seed);
        for _ in 0..=old_end {
            expected.next_u64();
        }
        let expected_values = (0..8).map(|_| expected.next_u64()).collect::<Vec<_>>();

        cache.ensure_through(requested_end, None).unwrap();

        assert!(cache.available_end() >= requested_end);
        assert_eq!(clone.available_end(), cache.available_end());
        assert_eq!(
            cache
                .read_values(old_end + 1, expected_values.len())
                .unwrap(),
            expected_values
        );
    }

    fn assert_cache_readable(cache: &ObservationCache) {
        let values = cache
            .read_values(HIGH_START, 4)
            .expect("read high-frame SFMT values");
        assert_eq!(values.len(), 4);
    }

    fn wait_until_removed(path: &Path) -> Duration {
        let started = Instant::now();
        for _ in 0..3_000 {
            if !path.exists() {
                return started.elapsed();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("cache file was not removed: {}", path.display());
    }

    #[test]
    #[ignore = "generates and writes a roughly 400 MiB cache to exercise high-frame reset races"]
    fn high_frame_reset_and_immediate_restart_keep_the_new_cache() {
        let first = generate_high_frame_cache();
        let cache_path = first.cache_file.path.clone();
        let cache_bytes = fs::metadata(&cache_path).expect("cache metadata").len();
        assert!(cache_bytes > 370 * 1024 * 1024);

        // Model the GUI flow: observation begins, Target changes, and reset releases its cache.
        let old_worker_owner = first.clone();
        let mut state = crate::application::state::AppState::default();
        state.start_observation_pending();
        state.apply_observation_cache(first);
        state.target = HIGH_END.to_string();
        state.cancel_observation();

        // Start another observation before assuming the asynchronous remover has finished.
        let restart_started = Instant::now();
        let second = generate_high_frame_cache();
        let restart_elapsed = restart_started.elapsed();
        let second_cache_path = second.cache_file.path.clone();
        assert_ne!(second_cache_path, cache_path);
        assert_cache_readable(&second);
        drop(old_worker_owner);

        // A reset cleanup arriving after the new cache has been acquired must preserve it.
        CacheStore::default().clear_unused_files();
        std::thread::sleep(Duration::from_millis(50));
        assert!(!cache_path.exists());
        assert!(second_cache_path.exists());
        assert_cache_readable(&second);
        eprintln!("immediate restart: {restart_elapsed:?}; cache remains readable after cleanup");

        // Model the opposite ordering: let cleanup finish first, then restart after 50 ms.
        drop(second);
        let delete_elapsed = wait_until_removed(&second_cache_path);
        std::thread::sleep(Duration::from_millis(50));
        let third = generate_high_frame_cache();
        let third_cache_path = third.cache_file.path.clone();
        assert_cache_readable(&third);
        assert!(third_cache_path.exists());
        eprintln!("delete-before-restart: {delete_elapsed:?}; cache regenerated and readable");

        // Race cleanup against an actual high-frame cache lookup/generation.
        drop(third);
        wait_until_removed(&third_cache_path);
        let barrier = Arc::new(Barrier::new(3));
        let cleanup_barrier = Arc::clone(&barrier);
        let cleanup = std::thread::spawn(move || {
            cleanup_barrier.wait();
            CacheStore::default().clear_unused_files_now();
        });
        let generation_barrier = Arc::clone(&barrier);
        let generation = std::thread::spawn(move || {
            generation_barrier.wait();
            generate_high_frame_cache()
        });
        barrier.wait();
        cleanup.join().expect("cleanup worker");
        let fourth = generation.join().expect("generation worker");
        let fourth_cache_path = fourth.cache_file.path.clone();
        std::thread::sleep(Duration::from_millis(50));
        assert!(fourth_cache_path.exists());
        assert_cache_readable(&fourth);
        eprintln!("concurrent cleanup/restart: new cache remains readable");

        drop(fourth);
        wait_until_removed(&fourth_cache_path);
    }
}
