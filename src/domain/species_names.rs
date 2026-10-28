//! Stable Gen 7 species-ID validation, independent of localized display text.

pub(crate) const SPECIES_COUNT: usize = 808;

pub(crate) fn is_supported_id(species_id: u16) -> bool {
    usize::from(species_id) < SPECIES_COUNT
}

#[cfg(test)]
mod tests {
    use super::{is_supported_id, SPECIES_COUNT};

    #[test]
    fn resource_preserves_all_species_ids() {
        assert_eq!(SPECIES_COUNT, 808);
        assert!(is_supported_id(0));
        assert!(is_supported_id(807));
        assert!(!is_supported_id(808));
    }
}
