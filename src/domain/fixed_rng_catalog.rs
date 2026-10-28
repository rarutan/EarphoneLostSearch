//! Generation 7 fixed RNG presets used by the Timeleap search form.
//!
//! Preset categories, species IDs, delay values, and the menu/dialogue classification
//! follow 3DSRNGTool's `PKM7.Species_SM` / `Species_USUM` data and personal-info rules.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GameEdition {
    SunMoon,
    #[default]
    UltraSunUltraMoon,
}

impl GameEdition {
    pub(crate) const ALL: [Self; 2] = [Self::SunMoon, Self::UltraSunUltraMoon];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::SunMoon => "SM",
            Self::UltraSunUltraMoon => "USUM",
        }
    }

    pub(crate) const fn courier_event_delay_frames(self) -> i16 {
        match self {
            Self::SunMoon => 62,
            Self::UltraSunUltraMoon => 42,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresetMode {
    Menu,
    Dialogue,
    Unsupported,
}

/// 3DSRNGTool `PKM7.DelayType` used by `RNGPool.StationaryDelay7`.
///
/// This is encounter-generator metadata, separate from the Timeleap trigger
/// (menu close, dialogue advance, or courier event).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum StationaryDelayType {
    #[default]
    Normal = 0,
    SunMoonSolgaleo = 1,
    SunMoonLunala = 2,
    SunMoonExeggutor = 3,
    SunMoonStarter = 5,
    Solgaleo = 9,
    Lunala = 10,
    Nihilego = 11,
    Buzzwole = 12,
    Pheromosa = 13,
    Xurkitree = 14,
    Celesteela = 15,
    Kartana = 16,
    Guzzlord = 17,
    FirstNihilego = 21,
    FirstBuzzwole = 22,
    FirstPheromosa = 23,
    FirstXurkitree = 24,
    FirstCelesteela = 25,
    FirstKartana = 26,
    FirstGuzzlord = 27,
    CommonLegendary = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixedRngPreset {
    pub species_id: u16,
    pub suffix: &'static str,
    pub delay_frames: i16,
    pub mode: PresetMode,
    pub delay_type: StationaryDelayType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixedRngCategory {
    pub name: &'static str,
    pub presets: &'static [FixedRngPreset],
}

const fn p(species_id: u16, delay_frames: i16, mode: PresetMode) -> FixedRngPreset {
    FixedRngPreset {
        species_id,
        suffix: "",
        delay_frames,
        mode,
        delay_type: StationaryDelayType::Normal,
    }
}

const fn pf(
    species_id: u16,
    suffix: &'static str,
    delay_frames: i16,
    mode: PresetMode,
) -> FixedRngPreset {
    FixedRngPreset {
        species_id,
        suffix,
        delay_frames,
        mode,
        delay_type: StationaryDelayType::Normal,
    }
}

const fn pt(
    species_id: u16,
    delay_frames: i16,
    mode: PresetMode,
    delay_type: StationaryDelayType,
) -> FixedRngPreset {
    FixedRngPreset {
        species_id,
        suffix: "",
        delay_frames,
        mode,
        delay_type,
    }
}

const MENU: PresetMode = PresetMode::Menu;
const DIALOGUE: PresetMode = PresetMode::Dialogue;
const UNSUPPORTED: PresetMode = PresetMode::Unsupported;

const EMPTY: &[FixedRngPreset] = &[p(0, 0, MENU)];
const ALOLA_LEGENDARY: &[FixedRngPreset] = &[
    p(785, 0, DIALOGUE),
    p(786, 0, DIALOGUE),
    p(787, 0, DIALOGUE),
    p(788, 0, DIALOGUE),
    p(789, 20, DIALOGUE),
    pt(791, 166, MENU, StationaryDelayType::Solgaleo),
    pt(792, 160, MENU, StationaryDelayType::Lunala),
    p(800, 0, DIALOGUE),
    pt(793, 134, MENU, StationaryDelayType::Nihilego),
    pt(794, 178, MENU, StationaryDelayType::Buzzwole),
    pt(795, 150, MENU, StationaryDelayType::Pheromosa),
    pt(796, 134, MENU, StationaryDelayType::Xurkitree),
    pt(797, 186, MENU, StationaryDelayType::Celesteela),
    pt(798, 130, MENU, StationaryDelayType::Kartana),
    pt(799, 124, MENU, StationaryDelayType::Guzzlord),
    p(772, 20, DIALOGUE),
    p(801, 20, DIALOGUE),
    p(803, 20, DIALOGUE),
];
const KANTO_LEGENDARY: &[FixedRngPreset] = &[
    pt(144, 150, MENU, StationaryDelayType::CommonLegendary),
    pt(145, 151, MENU, StationaryDelayType::CommonLegendary),
    pt(146, 150, MENU, StationaryDelayType::CommonLegendary),
    pt(150, 156, MENU, StationaryDelayType::CommonLegendary),
];
const JOHTO_LEGENDARY: &[FixedRngPreset] = &[
    pt(243, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(244, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(245, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(249, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(250, 150, MENU, StationaryDelayType::CommonLegendary),
];
const HOENN_LEGENDARY: &[FixedRngPreset] = &[
    pt(377, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(378, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(379, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(380, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(381, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(382, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(383, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(384, 150, MENU, StationaryDelayType::CommonLegendary),
];
const SINNOH_LEGENDARY: &[FixedRngPreset] = &[
    pt(480, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(481, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(482, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(483, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(484, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(485, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(486, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(487, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(488, 150, MENU, StationaryDelayType::CommonLegendary),
];
const UNOVA_LEGENDARY: &[FixedRngPreset] = &[
    pt(638, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(639, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(640, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(641, 150, MENU, StationaryDelayType::CommonLegendary),
    pt(642, 150, MENU, StationaryDelayType::CommonLegendary),
    pt(645, 150, MENU, StationaryDelayType::CommonLegendary),
    pt(643, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(644, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(646, 156, MENU, StationaryDelayType::CommonLegendary),
];
const KALOS_LEGENDARY: &[FixedRngPreset] = &[
    pt(716, 156, MENU, StationaryDelayType::CommonLegendary),
    pt(717, 150, MENU, StationaryDelayType::CommonLegendary),
    p(718, 0, DIALOGUE),
    pf(718, " 10%", 20, DIALOGUE),
    pf(718, " 10%", 22, DIALOGUE),
    pf(718, " 50%", 22, DIALOGUE),
];
const ULTRA_SPACE: &[FixedRngPreset] = &[
    pt(334, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(469, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(561, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(581, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(277, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(452, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(531, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(695, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(274, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(326, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(460, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(308, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(450, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(558, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(219, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(689, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(271, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(618, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(419, 94, MENU, StationaryDelayType::CommonLegendary),
    pt(195, 94, MENU, StationaryDelayType::CommonLegendary),
];
const TOTEM: &[FixedRngPreset] = &[
    p(735, 20, DIALOGUE),
    p(20, 20, DIALOGUE),
    p(105, 20, DIALOGUE),
    p(752, 20, DIALOGUE),
    p(754, 20, DIALOGUE),
    p(758, 20, DIALOGUE),
    p(738, 20, DIALOGUE),
    p(777, 20, DIALOGUE),
    p(778, 20, DIALOGUE),
    p(743, 20, DIALOGUE),
    p(784, 20, DIALOGUE),
];
const STARTERS: &[FixedRngPreset] = &[
    p(722, 20, DIALOGUE),
    p(725, 20, DIALOGUE),
    p(728, 20, DIALOGUE),
];
const NORMAL_STATIONARY: &[FixedRngPreset] = &[
    p(736, 12, DIALOGUE),
    p(92, 0, DIALOGUE),
    p(425, 0, DIALOGUE),
    p(760, 0, DIALOGUE),
    p(97, 102, DIALOGUE),
    pf(132, " Route 9", 0, DIALOGUE),
    pf(132, " Konikoni 1", 0, DIALOGUE),
    pf(132, " Konikoni 2", 0, DIALOGUE),
    pf(132, " Konikoni 3", 0, DIALOGUE),
    pf(132, " Konikoni 4", 0, DIALOGUE),
    p(592, -4, DIALOGUE),
    p(769, 0, DIALOGUE),
    p(127, 0, MENU),
    p(101, 152, MENU),
];
const IN_GAME_GIFT: &[FixedRngPreset] = &[
    pf(25, "（波乗り）", 20, DIALOGUE),
    pf(25, "（サトピカ）", 20, DIALOGUE),
    p(137, 20, DIALOGUE),
    p(142, 20, DIALOGUE),
    p(133, 20, DIALOGUE),
];
const FOSSILS: &[FixedRngPreset] = &[
    p(408, 26, DIALOGUE),
    p(410, 26, DIALOGUE),
    p(564, 26, DIALOGUE),
    p(566, 26, DIALOGUE),
    p(138, 26, DIALOGUE),
    p(140, 26, DIALOGUE),
    p(142, 26, DIALOGUE),
    p(345, 26, DIALOGUE),
    p(347, 26, DIALOGUE),
    p(696, 26, DIALOGUE),
    p(698, 26, DIALOGUE),
];
const IN_GAME_TRADE: &[FixedRngPreset] = &[
    p(701, 4, DIALOGUE),
    p(714, 4, DIALOGUE),
    p(339, 4, DIALOGUE),
    p(24, 4, DIALOGUE),
    p(709, 4, DIALOGUE),
    p(422, 4, DIALOGUE),
    p(128, 4, DIALOGUE),
];
const UB_FIRST: &[FixedRngPreset] = &[
    pt(793, 702, MENU, StationaryDelayType::FirstNihilego),
    pt(794, 1654, MENU, StationaryDelayType::FirstBuzzwole),
    pt(795, 150, MENU, StationaryDelayType::FirstPheromosa),
    pt(796, 958, MENU, StationaryDelayType::FirstXurkitree),
    pt(797, 1170, MENU, StationaryDelayType::FirstCelesteela),
    pt(798, 1618, MENU, StationaryDelayType::FirstKartana),
    pt(799, 968, MENU, StationaryDelayType::FirstGuzzlord),
];
const POKE_PELAGO: &[FixedRngPreset] = &[
    p(731, 8, UNSUPPORTED),
    p(278, 8, UNSUPPORTED),
    p(41, 8, UNSUPPORTED),
    p(742, 8, UNSUPPORTED),
    p(86, 8, UNSUPPORTED),
    p(79, 8, UNSUPPORTED),
    p(120, 8, UNSUPPORTED),
    p(222, 8, UNSUPPORTED),
    p(122, 8, UNSUPPORTED),
    p(180, 8, UNSUPPORTED),
    p(124, 8, UNSUPPORTED),
    p(127, 8, UNSUPPORTED),
    p(177, 8, UNSUPPORTED),
    p(764, 8, UNSUPPORTED),
    p(163, 8, UNSUPPORTED),
    p(771, 8, UNSUPPORTED),
    p(701, 8, UNSUPPORTED),
    p(131, 8, UNSUPPORTED),
    p(354, 8, UNSUPPORTED),
    p(200, 8, UNSUPPORTED),
    p(209, 8, UNSUPPORTED),
    p(667, 8, UNSUPPORTED),
    p(357, 8, UNSUPPORTED),
    p(430, 8, UNSUPPORTED),
    p(228, 8, UNSUPPORTED),
    p(309, 8, UNSUPPORTED),
];

const SM_LEGENDARY: &[FixedRngPreset] = &[
    p(785, 0, DIALOGUE),
    p(786, 0, DIALOGUE),
    p(787, 0, DIALOGUE),
    p(788, 0, DIALOGUE),
    pt(791, 288, MENU, StationaryDelayType::SunMoonSolgaleo),
    pt(792, 282, MENU, StationaryDelayType::SunMoonLunala),
    p(789, 34, DIALOGUE),
    p(772, 34, DIALOGUE),
    p(801, 34, DIALOGUE),
    pf(718, "（10%）", 32, DIALOGUE),
    pf(718, "（50%）", 32, DIALOGUE),
];
const SM_IN_GAME_GIFT: &[FixedRngPreset] = &[
    p(142, 34, DIALOGUE),
    p(137, 34, DIALOGUE),
    p(133, 4, DIALOGUE),
];
const SM_NORMAL_STATIONARY: &[FixedRngPreset] = &[
    p(731, 16, DIALOGUE),
    pt(103, 88, MENU, StationaryDelayType::SunMoonExeggutor),
];
const SM_STARTERS: &[FixedRngPreset] = &[
    pt(722, 40, MENU, StationaryDelayType::SunMoonStarter),
    pt(725, 40, MENU, StationaryDelayType::SunMoonStarter),
    pt(728, 40, MENU, StationaryDelayType::SunMoonStarter),
];
const SM_IN_GAME_TRADE: &[FixedRngPreset] = &[
    p(66, 4, DIALOGUE),
    p(761, 4, DIALOGUE),
    p(61, 4, DIALOGUE),
    p(440, 4, DIALOGUE),
    pf(76, "（アローラのすがた）", 4, DIALOGUE),
    p(762, 4, DIALOGUE),
    p(663, 4, DIALOGUE),
];
const SM_POKE_PELAGO: &[FixedRngPreset] = &[
    p(21, 8, UNSUPPORTED),
    p(41, 8, UNSUPPORTED),
    p(90, 8, UNSUPPORTED),
    p(278, 8, UNSUPPORTED),
    p(731, 8, UNSUPPORTED),
    p(64, 8, UNSUPPORTED),
    p(81, 8, UNSUPPORTED),
    p(92, 8, UNSUPPORTED),
    p(198, 8, UNSUPPORTED),
    p(426, 8, UNSUPPORTED),
    p(703, 8, UNSUPPORTED),
    p(60, 8, UNSUPPORTED),
    p(120, 8, UNSUPPORTED),
    p(127, 8, UNSUPPORTED),
    p(661, 8, UNSUPPORTED),
    p(709, 8, UNSUPPORTED),
    p(771, 8, UNSUPPORTED),
    p(227, 8, UNSUPPORTED),
    p(375, 8, UNSUPPORTED),
    p(707, 8, UNSUPPORTED),
    p(123, 8, UNSUPPORTED),
    p(131, 8, UNSUPPORTED),
    p(429, 8, UNSUPPORTED),
    p(587, 8, UNSUPPORTED),
    p(627, 8, UNSUPPORTED),
    p(629, 8, UNSUPPORTED),
];
const SM_FOSSILS: &[FixedRngPreset] = &[
    p(408, 40, DIALOGUE),
    p(410, 40, DIALOGUE),
    p(564, 40, DIALOGUE),
    p(566, 40, DIALOGUE),
    p(138, 40, DIALOGUE),
    p(140, 40, DIALOGUE),
    p(142, 40, DIALOGUE),
    p(345, 40, DIALOGUE),
    p(347, 40, DIALOGUE),
    p(696, 40, DIALOGUE),
    p(698, 40, DIALOGUE),
];

pub(crate) const SM_CATEGORIES: &[FixedRngCategory] = &[
    FixedRngCategory {
        name: "-",
        presets: EMPTY,
    },
    FixedRngCategory {
        name: "Legendary",
        presets: SM_LEGENDARY,
    },
    FixedRngCategory {
        name: "In-Game Gift",
        presets: SM_IN_GAME_GIFT,
    },
    FixedRngCategory {
        name: "Normal Stationary",
        presets: SM_NORMAL_STATIONARY,
    },
    FixedRngCategory {
        name: "Starters",
        presets: SM_STARTERS,
    },
    FixedRngCategory {
        name: "In-Game Trade",
        presets: SM_IN_GAME_TRADE,
    },
    FixedRngCategory {
        name: "Poke Pelago",
        presets: SM_POKE_PELAGO,
    },
    FixedRngCategory {
        name: "Fossils",
        presets: SM_FOSSILS,
    },
];

pub(crate) const fn categories(edition: GameEdition) -> &'static [FixedRngCategory] {
    match edition {
        GameEdition::SunMoon => SM_CATEGORIES,
        GameEdition::UltraSunUltraMoon => CATEGORIES,
    }
}

pub(crate) fn selected_preset_for(
    edition: GameEdition,
    category: usize,
    preset: usize,
) -> Option<&'static FixedRngPreset> {
    categories(edition).get(category)?.presets.get(preset)
}

pub(crate) fn normalize_selection(
    edition: GameEdition,
    category: usize,
    preset: usize,
) -> (usize, usize) {
    let catalogs = categories(edition);
    if category == catalogs.len() {
        return (catalogs.len(), 0);
    }
    let Some(selected_category) = catalogs.get(category) else {
        return (0, 0);
    };
    if preset < selected_category.presets.len() {
        (category, preset)
    } else {
        (category, 0)
    }
}

pub(crate) fn find_unique_preset(
    edition: GameEdition,
    species_id: u16,
    suffix: &str,
) -> Option<(usize, usize)> {
    let mut found = None;
    for (category_index, category) in categories(edition).iter().enumerate() {
        for (preset_index, preset) in category.presets.iter().enumerate() {
            if preset.species_id == species_id && preset.suffix == suffix {
                if found.is_some() {
                    return None;
                }
                found = Some((category_index, preset_index));
            }
        }
    }
    found
}

pub(crate) const CATEGORIES: &[FixedRngCategory] = &[
    FixedRngCategory {
        name: "-",
        presets: EMPTY,
    },
    FixedRngCategory {
        name: "Alola Legendary",
        presets: ALOLA_LEGENDARY,
    },
    FixedRngCategory {
        name: "Kanto Legendary",
        presets: KANTO_LEGENDARY,
    },
    FixedRngCategory {
        name: "Johto Legendary",
        presets: JOHTO_LEGENDARY,
    },
    FixedRngCategory {
        name: "Hoenn Legendary",
        presets: HOENN_LEGENDARY,
    },
    FixedRngCategory {
        name: "Sinnoh Legendary",
        presets: SINNOH_LEGENDARY,
    },
    FixedRngCategory {
        name: "Unova Legendary",
        presets: UNOVA_LEGENDARY,
    },
    FixedRngCategory {
        name: "Kalos Legendary",
        presets: KALOS_LEGENDARY,
    },
    FixedRngCategory {
        name: "Ultra Space Wilds",
        presets: ULTRA_SPACE,
    },
    FixedRngCategory {
        name: "Totem",
        presets: TOTEM,
    },
    FixedRngCategory {
        name: "Starters",
        presets: STARTERS,
    },
    FixedRngCategory {
        name: "Normal Stationary",
        presets: NORMAL_STATIONARY,
    },
    FixedRngCategory {
        name: "In-Game Gift",
        presets: IN_GAME_GIFT,
    },
    FixedRngCategory {
        name: "Fossils",
        presets: FOSSILS,
    },
    FixedRngCategory {
        name: "In-Game Trade",
        presets: IN_GAME_TRADE,
    },
    FixedRngCategory {
        name: "UB (First Encounter)",
        presets: UB_FIRST,
    },
    FixedRngCategory {
        name: "Poke Pelago",
        presets: POKE_PELAGO,
    },
];

#[cfg(test)]
pub(crate) fn selected_preset(category: usize, preset: usize) -> Option<&'static FixedRngPreset> {
    CATEGORIES.get(category)?.presets.get(preset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rng_categories_match_usum_source_order() {
        assert_eq!(CATEGORIES[1].name, "Alola Legendary");
        assert_eq!(CATEGORIES[11].name, "Normal Stationary");
        assert_eq!(CATEGORIES[15].name, "UB (First Encounter)");
        assert_eq!(CATEGORIES[16].name, "Poke Pelago");
    }

    #[test]
    fn source_flags_select_menu_and_pelago_does_not_support_timeleap() {
        assert_eq!(selected_preset(1, 5).unwrap().mode, PresetMode::Menu);
        assert_eq!(selected_preset(1, 4).unwrap().mode, PresetMode::Dialogue);
        assert_eq!(
            selected_preset(16, 0).unwrap().mode,
            PresetMode::Unsupported
        );
    }

    #[test]
    fn representative_presets_match_3dsrngtool_delay_and_method_metadata() {
        let tapu_koko = selected_preset(1, 0).unwrap();
        assert_eq!(tapu_koko.species_id, 785);
        assert_eq!(tapu_koko.delay_frames, 0);
        assert_eq!(tapu_koko.mode, PresetMode::Dialogue);
        assert_eq!(tapu_koko.delay_type, StationaryDelayType::Normal);

        let cosmog = selected_preset(1, 4).unwrap();
        assert_eq!(cosmog.delay_frames, 20);
        assert_eq!(cosmog.mode, PresetMode::Dialogue);

        let articuno = selected_preset(2, 0).unwrap();
        assert_eq!(articuno.delay_frames, 150);
        assert_eq!(articuno.mode, PresetMode::Menu);
        assert_eq!(articuno.delay_type, StationaryDelayType::CommonLegendary);

        let ultra_beast = selected_preset(15, 1).unwrap();
        assert_eq!(ultra_beast.delay_frames, 1654);
        assert_eq!(ultra_beast.mode, PresetMode::Menu);
        assert_eq!(ultra_beast.delay_type, StationaryDelayType::FirstBuzzwole);

        let frillish = selected_preset(11, 10).unwrap();
        assert_eq!(frillish.delay_frames, -4);
        assert_eq!(frillish.mode, PresetMode::Dialogue);
    }

    #[test]
    fn category_metadata_contains_all_source_entries() {
        assert_eq!(
            CATEGORIES
                .iter()
                .map(|category| category.presets.len())
                .sum::<usize>(),
            164
        );
    }

    #[test]
    fn pikachu_gift_presets_keep_the_species_id() {
        let in_game_gift = CATEGORIES
            .iter()
            .find(|category| category.name == "In-Game Gift")
            .unwrap();

        assert_eq!(in_game_gift.presets[0].species_id, 25);
        assert_eq!(in_game_gift.presets[1].species_id, 25);
    }

    #[test]
    fn sun_moon_categories_and_preset_counts_match_source_order() {
        assert_eq!(
            SM_CATEGORIES
                .iter()
                .map(|category| category.name)
                .collect::<Vec<_>>(),
            [
                "-",
                "Legendary",
                "In-Game Gift",
                "Normal Stationary",
                "Starters",
                "In-Game Trade",
                "Poke Pelago",
                "Fossils",
            ]
        );
        assert_eq!(
            SM_CATEGORIES
                .iter()
                .map(|category| category.presets.len())
                .sum::<usize>(),
            64 // 63 selectable source presets plus the neutral placeholder.
        );
    }

    #[test]
    fn sun_moon_full_catalog_metadata_matches_the_reference_manifest() {
        type Entry = (u16, &'static str, i16, PresetMode, StationaryDelayType);
        let expected: [&[Entry]; 8] = [
            &[(0, "", 0, MENU, StationaryDelayType::Normal)],
            &[
                (785, "", 0, DIALOGUE, StationaryDelayType::Normal),
                (786, "", 0, DIALOGUE, StationaryDelayType::Normal),
                (787, "", 0, DIALOGUE, StationaryDelayType::Normal),
                (788, "", 0, DIALOGUE, StationaryDelayType::Normal),
                (791, "", 288, MENU, StationaryDelayType::SunMoonSolgaleo),
                (792, "", 282, MENU, StationaryDelayType::SunMoonLunala),
                (789, "", 34, DIALOGUE, StationaryDelayType::Normal),
                (772, "", 34, DIALOGUE, StationaryDelayType::Normal),
                (801, "", 34, DIALOGUE, StationaryDelayType::Normal),
                (718, "（10%）", 32, DIALOGUE, StationaryDelayType::Normal),
                (718, "（50%）", 32, DIALOGUE, StationaryDelayType::Normal),
            ],
            &[
                (142, "", 34, DIALOGUE, StationaryDelayType::Normal),
                (137, "", 34, DIALOGUE, StationaryDelayType::Normal),
                (133, "", 4, DIALOGUE, StationaryDelayType::Normal),
            ],
            &[
                (731, "", 16, DIALOGUE, StationaryDelayType::Normal),
                (103, "", 88, MENU, StationaryDelayType::SunMoonExeggutor),
            ],
            &[
                (722, "", 40, MENU, StationaryDelayType::SunMoonStarter),
                (725, "", 40, MENU, StationaryDelayType::SunMoonStarter),
                (728, "", 40, MENU, StationaryDelayType::SunMoonStarter),
            ],
            &[
                (66, "", 4, DIALOGUE, StationaryDelayType::Normal),
                (761, "", 4, DIALOGUE, StationaryDelayType::Normal),
                (61, "", 4, DIALOGUE, StationaryDelayType::Normal),
                (440, "", 4, DIALOGUE, StationaryDelayType::Normal),
                (
                    76,
                    "（アローラのすがた）",
                    4,
                    DIALOGUE,
                    StationaryDelayType::Normal,
                ),
                (762, "", 4, DIALOGUE, StationaryDelayType::Normal),
                (663, "", 4, DIALOGUE, StationaryDelayType::Normal),
            ],
            &[
                (21, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (41, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (90, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (278, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (731, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (64, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (81, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (92, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (198, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (426, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (703, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (60, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (120, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (127, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (661, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (709, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (771, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (227, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (375, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (707, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (123, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (131, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (429, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (587, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (627, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
                (629, "", 8, UNSUPPORTED, StationaryDelayType::Normal),
            ],
            &[
                (408, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (410, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (564, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (566, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (138, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (140, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (142, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (345, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (347, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (696, "", 40, DIALOGUE, StationaryDelayType::Normal),
                (698, "", 40, DIALOGUE, StationaryDelayType::Normal),
            ],
        ];

        for (category, expected_entries) in SM_CATEGORIES.iter().zip(expected) {
            let actual = category
                .presets
                .iter()
                .map(|preset| {
                    (
                        preset.species_id,
                        preset.suffix,
                        preset.delay_frames,
                        preset.mode,
                        preset.delay_type,
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected_entries, "category {}", category.name);
        }
    }

    #[test]
    fn sun_moon_delay_metadata_matches_reference_entries() {
        let solgaleo = selected_preset_for(GameEdition::SunMoon, 1, 4).unwrap();
        assert_eq!(solgaleo.species_id, 791);
        assert_eq!(solgaleo.delay_frames, 288);
        assert_eq!(solgaleo.mode, PresetMode::Menu);
        assert_eq!(solgaleo.delay_type, StationaryDelayType::SunMoonSolgaleo);

        let lunala = selected_preset_for(GameEdition::SunMoon, 1, 5).unwrap();
        assert_eq!(lunala.delay_frames, 282);
        assert_eq!(lunala.delay_type, StationaryDelayType::SunMoonLunala);

        let exeggutor = selected_preset_for(GameEdition::SunMoon, 3, 1).unwrap();
        assert_eq!(exeggutor.delay_frames, 88);
        assert_eq!(exeggutor.delay_type, StationaryDelayType::SunMoonExeggutor);

        let starter = selected_preset_for(GameEdition::SunMoon, 4, 0).unwrap();
        assert_eq!(starter.species_id, 722);
        assert_eq!(starter.delay_frames, 40);
        assert_eq!(starter.delay_type, StationaryDelayType::SunMoonStarter);

        assert_eq!(
            selected_preset_for(GameEdition::SunMoon, 6, 0)
                .unwrap()
                .mode,
            PresetMode::Unsupported
        );
    }

    #[test]
    fn edition_lookup_defaults_to_usum_and_has_version_specific_event_delays() {
        assert_eq!(GameEdition::default(), GameEdition::UltraSunUltraMoon);
        assert_eq!(GameEdition::SunMoon.courier_event_delay_frames(), 62);
        assert_eq!(
            GameEdition::UltraSunUltraMoon.courier_event_delay_frames(),
            42
        );
        assert_eq!(categories(GameEdition::SunMoon), SM_CATEGORIES);
        assert_eq!(categories(GameEdition::UltraSunUltraMoon), CATEGORIES);
    }

    #[test]
    fn encounter_mapping_only_returns_unique_edition_matches() {
        assert_eq!(
            find_unique_preset(GameEdition::SunMoon, 791, ""),
            Some((1, 4))
        );
        assert_eq!(find_unique_preset(GameEdition::SunMoon, 999, ""), None);
        assert_eq!(find_unique_preset(GameEdition::SunMoon, 718, ""), None);
    }
}
