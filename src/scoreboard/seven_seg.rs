//! Seven-segment decoding. Bit order: a=0, b=1, c=2, d=3, e=4, f=5, g=6.
//!
//! ```text
//!  aaa
//! f   b
//!  ggg
//! e   c
//!  ddd
//! ```

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Digit {
    Blank,
    Num(u8),
    Unknown,
}

/// (mask, digit). Several entries per digit cover the variants (6/7/9 with and without the extra segment).
const PATTERNS: &[(u8, u8)] = &[
    (0x3F, 0),
    (0x06, 1),
    (0x5B, 2),
    (0x4F, 3),
    (0x66, 4),
    (0x6D, 5),
    (0x7D, 6),
    (0x7C, 6), // 6 without the top bar
    (0x07, 7),
    (0x27, 7), // 7 with f
    (0x7F, 8),
    (0x6F, 9),
    (0x67, 9), // 9 without the bottom bar
];

pub fn decode(mask: u8) -> Digit {
    if mask == 0 {
        return Digit::Blank;
    }
    PATTERNS
        .iter()
        .find(|(m, _)| *m == mask)
        .map(|(_, d)| Digit::Num(*d))
        .unwrap_or(Digit::Unknown)
}

/// Mask of a segment list such as `"abcdg"` (tests and synthetic boards).
pub fn mask_of(s: &str) -> u8 {
    s.chars().fold(0u8, |m, c| m | (1 << (c as u8 - b'a')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_digits() {
        let cases = [
            ("abcdef", 0),
            ("bc", 1),
            ("abdeg", 2),
            ("abcdg", 3),
            ("bcfg", 4),
            ("acdfg", 5),
            ("acdefg", 6),
            ("abc", 7),
            ("abcdefg", 8),
            ("abcdfg", 9),
        ];
        for (s, d) in cases {
            assert_eq!(decode(mask_of(s)), Digit::Num(d), "{s}");
        }
    }

    #[test]
    fn blank_and_unknown() {
        assert_eq!(decode(0), Digit::Blank);
        assert_eq!(decode(mask_of("a")), Digit::Unknown);
        assert_eq!(decode(mask_of("abcg")), Digit::Unknown);
    }
}
