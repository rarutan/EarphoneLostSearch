//! Minimal WC7/WC7FULL reader for the Event RNG settings used by Timeleap.

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WondercardSettings {
    pub(crate) tid: String,
    pub(crate) sid: String,
    pub(crate) ec: String,
    pub(crate) pid: String,
    pub(crate) pid_type: u8,
    pub(crate) fixed_ivs: [i8; 6],
    pub(crate) iv_count: String,
    pub(crate) ability_locked: bool,
    pub(crate) ability: u8,
    pub(crate) nature_locked: bool,
    pub(crate) nature: u8,
    pub(crate) gender_locked: bool,
    pub(crate) gender: u8,
    pub(crate) your_id: bool,
    pub(crate) is_egg: bool,
    pub(crate) species_id: u16,
}

pub(crate) fn parse(data: &[u8]) -> Result<WondercardSettings, String> {
    let base = match data.len() {
        264 => 0,
        // WC7FULL has 0x208 bytes of card metadata before the WC7 payload.
        784 => 0x208,
        length => return Err(format!("WC7のサイズが不正です: {length} bytes")),
    };
    let get = |offset: usize| data.get(base + offset).copied();
    let get_u16 = |offset: usize| {
        let start = base + offset;
        data.get(start..start + 2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let get_u32 = |offset: usize| {
        let start = base + offset;
        data.get(start..start + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let tid = get_u16(0x68).ok_or("TIDを読み込めませんでした。")?;
    let sid = get_u16(0x6a).ok_or("SIDを読み込めませんでした。")?;
    let species_id = get_u16(0x82).ok_or("ポケモン番号を読み込めませんでした。")?;
    if !crate::domain::species_names::is_supported_id(species_id) {
        return Err(format!("未対応のポケモン番号です: {species_id}"));
    }
    let ec = get_u32(0x70).ok_or("ECを読み込めませんでした。")?;
    let nature = get(0xa0).ok_or("性格を読み込めませんでした。")?;
    let gender = get(0xa1).ok_or("性別を読み込めませんでした。")?;
    let ability = get(0xa2).ok_or("特性を読み込めませんでした。")?;
    let pid_type = get(0xa3).ok_or("PIDタイプを読み込めませんでした。")?;
    // The PID payload is meaningful only for the Specified PID mode.
    let pid = if pid_type == 0 {
        get_u32(0xd4).ok_or("PIDを読み込めませんでした。")?
    } else {
        0
    };
    let mut fixed_ivs = [-1; 6];
    let mut iv_marker = 0u8;
    let mut raw_ivs = [0u8; 6];
    for (index, slot) in [0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4].into_iter().enumerate() {
        let value = get(slot).ok_or("IVを読み込めませんでした。")?;
        raw_ivs[index] = value;
        if (0xfc..=0xfe).contains(&value) {
            iv_marker = iv_marker.max(value);
        }
    }
    // A FC/FD/FE marker denotes a guaranteed-IV count; when present, the
    // remaining IV bytes are not treated as explicit fixed IV values.
    if iv_marker == 0 {
        for (index, value) in raw_ivs.into_iter().enumerate() {
            if value <= 31 {
                // WC7 stores IVs in the same HP/Atk/Def/SpA/SpD/Spe order as
                // the Event Setting UI. Do not use the internal PKM speed-first
                // order here.
                fixed_ivs[[0, 1, 2, 3, 5, 4][index]] = value as i8;
            }
        }
    }
    // The guaranteed-IV count comes from the marker in the IV area. YourID
    // is a separate field stored at 0xB5.
    let iv_count = iv_marker.saturating_sub(0xfb);
    Ok(WondercardSettings {
        tid: tid.to_string(),
        sid: sid.to_string(),
        ec: format!("{ec:08X}"),
        pid: format!("{pid:08X}"),
        pid_type: match pid_type {
            0 => 3,
            1 => 0,
            2 => 2,
            3 => 1,
            _ => return Err(format!("未知のPIDタイプです: {pid_type}")),
        },
        fixed_ivs,
        iv_count: iv_count.to_string(),
        ability_locked: ability <= 2,
        ability: match ability {
            0 => 1,
            1 => 2,
            2 => 3,
            _ => 1,
        },
        nature_locked: nature != 0xff,
        nature: nature.min(24),
        gender_locked: gender <= 2,
        gender: match gender {
            0 => 1, // male
            1 => 2, // female
            2 => 0, // genderless
            _ => 0,
        },
        your_id: get(0xb5).unwrap_or_default() == 3,
        is_egg: get(0xd1).unwrap_or_default() == 1,
        species_id,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_wc7_offsets() {
        let mut data = vec![0; 264];
        data[0x68..0x6a].copy_from_slice(&123u16.to_le_bytes());
        data[0x6a..0x6c].copy_from_slice(&456u16.to_le_bytes());
        data[0x70..0x74].copy_from_slice(&0x21u32.to_le_bytes());
        data[0x82..0x84].copy_from_slice(&803u16.to_le_bytes());
        data[0xaf] = 31;
        data[0xb0] = 30;
        data[0xb1] = 29;
        data[0xb2] = 28;
        data[0xb3] = 27;
        data[0xb4] = 26;
        data[0xb5] = 3;
        data[0xa3] = 3;
        data[0xa1] = 2;
        let parsed = parse(&data).unwrap();
        assert_eq!(parsed.tid, "123");
        assert_eq!(parsed.sid, "456");
        assert_eq!(parsed.ec, "00000021");
        assert_eq!(parsed.pid, "00000000");
        assert_eq!(parsed.species_id, 803);
        assert_eq!(parsed.fixed_ivs[0], 31);
        assert_eq!(parsed.fixed_ivs, [31, 30, 29, 28, 26, 27]);
        assert_eq!(parsed.iv_count, "0");
        assert!(parsed.gender_locked);
        assert_eq!(parsed.gender, 0);
        assert!(parsed.your_id);
        assert!(!parsed.is_egg);
    }

    #[test]
    fn parses_wc7full_identity_flags_relative_to_payload() {
        let base = 0x208;
        let mut data = vec![0; 784];
        data[base + 0x82..base + 0x84].copy_from_slice(&803u16.to_le_bytes());
        data[base + 0xb5] = 3;
        data[base + 0xd1] = 1;

        let parsed = parse(&data).unwrap();
        assert!(parsed.your_id);
        assert!(parsed.is_egg);
    }

    #[test]
    fn parses_non_your_id_wc7_values_as_false() {
        for flag in [0, 1, 2, 4, 255] {
            let mut data = vec![0; 264];
            data[0x82..0x84].copy_from_slice(&803u16.to_le_bytes());
            data[0xb5] = flag;

            assert!(!parse(&data).unwrap().your_id, "flag={flag}");
        }
    }

    #[test]
    fn parses_egg_flag_only_when_wc7_value_is_one() {
        let mut data = vec![0; 264];
        data[0x82..0x84].copy_from_slice(&803u16.to_le_bytes());
        data[0xd1] = 2;

        assert!(!parse(&data).unwrap().is_egg);
        data[0xd1] = 1;
        assert!(parse(&data).unwrap().is_egg);
    }

    #[test]
    fn converts_three_flawless_marker_to_guaranteed_count() {
        let mut data = vec![0; 264];
        data[0xaf] = 0xfe;
        data[0xb0..=0xb4].fill(0x1f);
        let parsed = parse(&data).unwrap();
        assert_eq!(parsed.iv_count, "3");
        assert_eq!(parsed.fixed_ivs, [-1; 6]);
    }

    #[test]
    fn keeps_unset_pid_and_ec_as_explicit_zero_hex_values() {
        let parsed = parse(&vec![0; 264]).unwrap();
        assert_eq!(parsed.pid, "00000000");
        assert_eq!(parsed.ec, "00000000");
    }

    #[test]
    fn ignores_pid_payload_for_non_specified_pid_types() {
        let mut data = vec![0; 264];
        data[0xa3] = 3;
        data[0xd4..0xd8].copy_from_slice(&0xee6b2802u32.to_le_bytes());
        let parsed = parse(&data).unwrap();
        assert_eq!(parsed.pid, "00000000");
        assert_eq!(parsed.pid_type, 1);
    }
}
