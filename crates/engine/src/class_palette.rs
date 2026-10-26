// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P15 contributor-original environment palette. RGB bytes are display data, never materials.

/// Original default environment preset, licensed MPL-2.0 with BeamCraft.
/// Class zero is slate; subsequent colours repeat for arbitrary int32 classes.
/// Negative classes use their unsigned magnitude, including `i32::MIN`.
pub const DEFAULT_CLASS_PALETTE: [[u8; 3]; 8] = [
    [126, 143, 159],
    [46, 173, 166],
    [225, 157, 65],
    [112, 139, 220],
    [190, 112, 167],
    [139, 175, 91],
    [218, 113, 98],
    [112, 182, 202],
];

/// Look up a class in an environment preset. An empty preset has no class colour.
pub fn class_colour(class: i32, palette: &[[u8; 3]]) -> Option<[u8; 3]> {
    let length = u64::try_from(palette.len()).ok()?;
    if length == 0 {
        return None;
    }
    let index = usize::try_from(u64::from(class.unsigned_abs()) % length).ok()?;
    palette.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_int32_class_has_a_colour_in_the_default_preset() {
        for class in [i32::MIN, -1, 0, 1, 8, i32::MAX] {
            assert!(class_colour(class, &DEFAULT_CLASS_PALETTE).is_some());
        }
        assert_eq!(class_colour(0, &[]), None);
        assert_eq!(class_colour(-1, &[[1, 2, 3]]), Some([1, 2, 3]));
    }
}
