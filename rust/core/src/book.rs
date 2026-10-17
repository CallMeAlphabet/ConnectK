//! Exact opening book with the same on-disk text format as the Java
//! implementation (books remain interchangeable):
//!
//! ```text
//! # connectk-book v1 depth=12
//! # rules: width=7 height=6 K=4 placement=GRAVITY directions=V,H,DU,DD misere=false ...
//! <current hex,16> <mask hex,16> <exact score> <canonical best column or -1>
//! ```
//!
//! Keys are mirror-canonical (single-word gravity rulesets only),
//! entries sorted by (current, mask) and looked up by binary search.
//! Every value in the book is an exact game value, so a hit is a proof.

use crate::model::{dirs, Placement, Rules};
use crate::position::Position;

pub const MISS: i32 = i32::MIN;

pub struct OpeningBook {
    keys: Vec<(u64, u64)>, // (current, mask), sorted
    scores: Vec<i32>,
    best_columns: Vec<i16>, // canonical-orientation column, -1 = unknown
    depth: i32,             // deepest ply covered, -1 = unknown (external books)
    min_ply: i32,
    max_ply: i32,
}

impl OpeningBook {
    /// Build from parallel arrays; must already be sorted by key with no
    /// duplicates (guaranteed by `bookgen`). `depth` is the ply bound the
    /// book was generated to.
    pub fn new(
        keys: Vec<(u64, u64)>,
        scores: Vec<i32>,
        best_columns: Vec<i16>,
        depth: i32,
    ) -> OpeningBook {
        let (mut lo, mut hi) = (i32::MAX, -1);
        for &(_, m) in &keys {
            let p = m.count_ones() as i32;
            lo = lo.min(p);
            hi = hi.max(p);
        }
        OpeningBook {
            keys,
            scores,
            best_columns,
            depth,
            min_ply: if hi < 0 { 0 } else { lo },
            max_ply: hi.max(0),
        }
    }

    pub fn size(&self) -> usize {
        self.keys.len()
    }
    pub fn depth(&self) -> i32 {
        self.depth
    }

    /// Cheap plausibility gate before probing: could this ply be covered?
    #[inline]
    pub fn covers_ply(&self, ply: i32) -> bool {
        if self.depth >= 0 {
            ply <= self.depth
        } else {
            ply >= self.min_ply && ply <= self.max_ply
        }
    }

    #[inline]
    pub fn probe(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        match self
            .keys
            .binary_search(&(canonical_current, canonical_mask))
        {
            Ok(i) => self.scores[i],
            Err(_) => MISS,
        }
    }

    /// Canonical-orientation best column stored for a key, or -1.
    pub fn best_column_canonical(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        match self
            .keys
            .binary_search(&(canonical_current, canonical_mask))
        {
            Ok(i) => self.best_columns[i] as i32,
            Err(_) => -1,
        }
    }

    /// Exact score for a canonical key, or `MISS` (the fast solver's API).
    #[inline]
    pub fn lookup_key(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        self.probe(canonical_current, canonical_mask)
    }

    /// Canonical best column for a key, or -1 (the fast solver's API).
    #[inline]
    pub fn best_col(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        self.best_column_canonical(canonical_current, canonical_mask)
    }

    /// Exact score for a position, or `MISS`. Caller ensures single-word
    /// rules (the book only supports those).
    pub fn lookup_position(&self, position: &Position) -> i32 {
        let rules = position.rules();
        if rules.word_count() != 1 || !self.covers_ply(position.moves_played() as i32) {
            return MISS;
        }
        let cur = position.current_words()[0];
        let mask = position.mask_words()[0];
        let (cc, mm) = canonical_single(cur, mask, rules);
        self.probe(cc, mm)
    }

    // ---- IO ----

    pub fn load(path: &std::path::Path) -> Result<OpeningBook, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read book {}: {e}", path.display()))?;
        OpeningBook::parse(&text)
    }

    pub fn parse(text: &str) -> Result<OpeningBook, String> {
        let mut depth = -1i32;
        let mut keys: Vec<(u64, u64)> = Vec::new();
        let mut scores: Vec<i32> = Vec::new();
        let mut best: Vec<i16> = Vec::new();
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                if let Some(i) = rest.find("depth=") {
                    let tail = &rest[i + 6..];
                    let num: String = tail
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '-')
                        .collect();
                    depth = num.parse().unwrap_or(-1);
                }
                continue;
            }
            let mut it = line.split_whitespace();
            let (Some(c), Some(m), Some(s)) = (it.next(), it.next(), it.next()) else {
                continue; // short lines are ignored, as in the Java loader
            };
            let cur = parse_hex(c, lineno)?;
            let mask = parse_hex(m, lineno)?;
            let score: i32 = s
                .parse()
                .map_err(|_| format!("bad score on book line {}", lineno + 1))?;
            let bc: i32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(-1);
            keys.push((cur, mask));
            scores.push(score);
            best.push(bc as i16);
        }
        // Sort parallel arrays by key (books we write are already sorted;
        // sorting here makes external/shuffled books safe too).
        let mut order: Vec<usize> = (0..keys.len()).collect();
        order.sort_by_key(|&i| keys[i]);
        let keys: Vec<(u64, u64)> = order.iter().map(|&i| keys[i]).collect();
        let scores: Vec<i32> = order.iter().map(|&i| scores[i]).collect();
        let best: Vec<i16> = order.iter().map(|&i| best[i]).collect();
        Ok(OpeningBook::new(keys, scores, best, depth))
    }

    pub fn save(&self, path: &std::path::Path, rules: &Rules) -> Result<(), String> {
        let mut s = String::with_capacity(self.keys.len() * 42 + 256);
        s.push_str(&format!("# connectk-book v1 depth={}\n", self.depth));
        s.push_str(&format!("# rules: {}\n", rules_line(rules)));
        for i in 0..self.keys.len() {
            let (c, m) = self.keys[i];
            s.push_str(&format!(
                "{c:016x} {m:016x} {} {}\n",
                self.scores[i], self.best_columns[i]
            ));
        }
        // Write-then-rename so an interrupted run never leaves a torn book.
        let tmp = path.with_extension("book.tmp");
        std::fs::write(&tmp, s)
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("cannot rename to {}: {e}", path.display()))?;
        Ok(())
    }
}

fn parse_hex(s: &str, lineno: usize) -> Result<u64, String> {
    u64::from_str_radix(s, 16).map_err(|_| format!("bad hex key on book line {}", lineno + 1))
}

pub fn rules_line(rules: &Rules) -> String {
    let mut names = Vec::new();
    if rules.directions & dirs::VERTICAL != 0 {
        names.push("V");
    }
    if rules.directions & dirs::HORIZONTAL != 0 {
        names.push("H");
    }
    if rules.directions & dirs::DIAGONAL_UP != 0 {
        names.push("DU");
    }
    if rules.directions & dirs::DIAGONAL_DOWN != 0 {
        names.push("DD");
    }
    format!(
        "width={} height={} K={} placement={} directions={} misere={}",
        rules.width,
        rules.height,
        rules.connect,
        if rules.placement == Placement::Gravity {
            "GRAVITY"
        } else {
            "FREE"
        },
        names.join(","),
        rules.misere
    )
}

/// Mirror-canonical (current, mask) for single-word boards, matching the
/// Java bit layout and tie-breaking exactly (mask compared first, then
/// current — the same order as `Position::canonical_key` and the fast
/// solver, so books stay interchangeable between implementations).
#[inline]
pub fn canonical_single(cur: u64, mask: u64, rules: &Rules) -> (u64, u64) {
    let (mc, mm, _) = canonical_single_flipped(cur, mask, rules);
    (mc, mm)
}

#[inline]
pub fn canonical_single_flipped(cur: u64, mask: u64, rules: &Rules) -> (u64, u64, bool) {
    if !rules.mirror_symmetric() {
        return (cur, mask, false);
    }
    let mm = mirror_word(mask, rules);
    if mm > mask {
        return (cur, mask, false);
    }
    let mc = mirror_word(cur, rules);
    if mm == mask && mc >= cur {
        return (cur, mask, false);
    }
    (mc, mm, true)
}

/// Left-right mirror of a single-word board. Column-wise bit math:
/// extract the `height` bits of each source column and place them at the
/// mirrored column offset.
#[inline]
pub fn mirror_word(bits: u64, rules: &Rules) -> u64 {
    let stride = rules.stride() as u32;
    let col_mask: u64 = (1u64 << rules.height) - 1;
    let mut out = 0u64;
    for c in 0..rules.width as u32 {
        let piece = (bits >> (c * stride)) & col_mask;
        out |= piece << ((rules.width as u32 - 1 - c) * stride);
    }
    out
}
