use crate::hash::{fast_map_with_capacity, mix_pair, FastMap};
use crate::model::{dirs, Move, Placement, Rules};
use crate::position::{Position, PositionKey};
use std::path::{Path, PathBuf};

pub const MISS: i32 = i32::MIN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BookEntry {
    pub score: i32,

    pub best_move: Option<Move>,
}

enum Storage {
    Single {
        keys: Vec<(u64, u64)>,
        scores: Vec<i32>,
        best_columns: Vec<i16>,
        index: Vec<u32>,
    },
    General {
        entries: FastMap<PositionKey, BookEntry>,
    },
}

pub struct OpeningBook {
    storage: Storage,
    depth: i32,
    min_ply: i32,
    max_ply: i32,
    rules: Option<Rules>,
}

fn ply_of_words(mask: &[u64]) -> i32 {
    mask.iter().map(|w| w.count_ones() as i32).sum()
}

fn single_storage(keys: Vec<(u64, u64)>, scores: Vec<i32>, best_columns: Vec<i16>) -> Storage {
    let index = build_single_index(&keys);
    Storage::Single {
        keys,
        scores,
        best_columns,
        index,
    }
}

fn build_single_index(keys: &[(u64, u64)]) -> Vec<u32> {
    if keys.len() < 4096 {
        return Vec::new();
    }
    let capacity = keys.len().saturating_mul(2).next_power_of_two().max(16);
    let mask = capacity - 1;
    let mut index = vec![0u32; capacity];
    for (i, &(cur, m)) in keys.iter().enumerate() {
        let mut slot = mix_pair(cur, m) as usize & mask;
        while index[slot] != 0 {
            slot = (slot + 1) & mask;
        }
        index[slot] = (i + 1) as u32;
    }
    index
}

#[inline]
fn find_single(keys: &[(u64, u64)], index: &[u32], key: (u64, u64)) -> Option<usize> {
    if index.is_empty() {
        return keys.binary_search(&key).ok();
    }
    let mask = index.len() - 1;
    let mut slot = mix_pair(key.0, key.1) as usize & mask;
    loop {
        let stored = index[slot];
        if stored == 0 {
            return None;
        }
        let i = (stored - 1) as usize;
        if keys[i] == key {
            return Some(i);
        }
        slot = (slot + 1) & mask;
    }
}

impl OpeningBook {
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
            storage: single_storage(keys, scores, best_columns),
            depth,
            min_ply: if hi < 0 { 0 } else { lo },
            max_ply: hi.max(0),
            rules: None,
        }
    }

    pub fn from_entries(
        entries: Vec<(PositionKey, BookEntry)>,
        depth: i32,
        rules: Option<Rules>,
    ) -> Result<OpeningBook, String> {
        let mut map: FastMap<PositionKey, BookEntry> =
            fast_map_with_capacity(entries.len().saturating_mul(2).max(16));
        let (mut lo, mut hi) = (i32::MAX, -1);
        for (key, entry) in entries {
            let p = ply_of_words(&key.mask);
            lo = lo.min(p);
            hi = hi.max(p);
            let replace = match map.get(&key) {
                Some(old) if old.score != entry.score => {
                    return Err(format!(
                        "conflicting book scores {} and {} for one position",
                        old.score, entry.score
                    ));
                }
                Some(old) => old.best_move.is_none() && entry.best_move.is_some(),
                None => true,
            };
            if replace {
                map.insert(key, entry);
            }
        }

        let can_single = rules
            .as_ref()
            .map(|r| r.placement == Placement::Gravity && r.word_count() == 1)
            .unwrap_or(false)
            && map
                .keys()
                .all(|k| k.current.len() == 1 && k.mask.len() == 1);
        let storage = if can_single {
            let mut rows: Vec<((u64, u64), i32, i16)> = map
                .into_iter()
                .map(|(k, e)| {
                    (
                        (k.current[0], k.mask[0]),
                        e.score,
                        e.best_move.map(|m| m.column as i16).unwrap_or(-1),
                    )
                })
                .collect();
            rows.sort_by_key(|r| r.0);
            single_storage(
                rows.iter().map(|r| r.0).collect(),
                rows.iter().map(|r| r.1).collect(),
                rows.iter().map(|r| r.2).collect(),
            )
        } else {
            Storage::General { entries: map }
        };

        Ok(OpeningBook {
            storage,
            depth,
            min_ply: if hi < 0 { 0 } else { lo },
            max_ply: hi.max(0),
            rules,
        })
    }

    pub fn merged(books: Vec<OpeningBook>) -> Result<OpeningBook, String> {
        let refs: Vec<&OpeningBook> = books.iter().collect();
        OpeningBook::merged_refs(&refs)
    }

    pub fn merged_refs(books: &[&OpeningBook]) -> Result<OpeningBook, String> {
        let mut rules: Option<Rules> = None;
        let mut depth = -1i32;
        let mut all: Vec<(PositionKey, BookEntry)> = Vec::new();
        for book in books {
            match (&rules, &book.rules) {
                (Some(a), Some(b)) if a != b => {
                    return Err("cannot merge books for different rules".into());
                }
                (None, Some(b)) => rules = Some(b.clone()),
                _ => {}
            }
            depth = depth.max(book.depth);
            all.extend(book.entries());
        }
        OpeningBook::from_entries(all, depth, rules)
    }

    pub fn with_rules(mut self, rules: Rules) -> OpeningBook {
        self.rules = Some(rules);
        self
    }

    pub fn size(&self) -> usize {
        match &self.storage {
            Storage::Single { keys, .. } => keys.len(),
            Storage::General { entries } => entries.len(),
        }
    }
    pub fn depth(&self) -> i32 {
        self.depth
    }
    pub fn min_ply(&self) -> i32 {
        self.min_ply
    }
    pub fn max_ply(&self) -> i32 {
        self.max_ply
    }
    pub fn rules(&self) -> Option<&Rules> {
        self.rules.as_ref()
    }

    #[inline]
    pub fn covers_ply(&self, ply: i32) -> bool {
        if self.depth >= 0 {
            ply <= self.depth
        } else {
            ply >= self.min_ply && ply <= self.max_ply
        }
    }

    #[inline]
    pub fn may_contain_ply(&self, ply: i32) -> bool {
        ply >= self.min_ply && ply <= self.max_ply
    }

    #[inline]
    pub fn probe(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        match &self.storage {
            Storage::Single {
                keys,
                scores,
                index,
                ..
            } => match find_single(keys, index, (canonical_current, canonical_mask)) {
                Some(i) => scores[i],
                None => MISS,
            },
            Storage::General { entries } => {
                let key = PositionKey {
                    current: Box::new([canonical_current]),
                    mask: Box::new([canonical_mask]),
                };
                entries.get(&key).map(|e| e.score).unwrap_or(MISS)
            }
        }
    }

    pub fn best_column_canonical(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        match &self.storage {
            Storage::Single {
                keys,
                best_columns,
                index,
                ..
            } => match find_single(keys, index, (canonical_current, canonical_mask)) {
                Some(i) => best_columns[i] as i32,
                None => -1,
            },
            Storage::General { entries } => {
                let key = PositionKey {
                    current: Box::new([canonical_current]),
                    mask: Box::new([canonical_mask]),
                };
                entries
                    .get(&key)
                    .and_then(|e| e.best_move)
                    .map(|m| m.column)
                    .unwrap_or(-1)
            }
        }
    }

    #[inline]
    pub fn lookup_key(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        self.probe(canonical_current, canonical_mask)
    }

    #[inline]
    pub fn best_col(&self, canonical_current: u64, canonical_mask: u64) -> i32 {
        self.best_column_canonical(canonical_current, canonical_mask)
    }

    #[inline]
    pub fn probe_entry(&self, canonical_current: u64, canonical_mask: u64) -> Option<(i32, i32)> {
        match &self.storage {
            Storage::Single {
                keys,
                scores,
                best_columns,
                index,
            } => find_single(keys, index, (canonical_current, canonical_mask))
                .map(|i| (scores[i], best_columns[i] as i32)),
            Storage::General { entries } => {
                let key = PositionKey {
                    current: Box::new([canonical_current]),
                    mask: Box::new([canonical_mask]),
                };
                entries
                    .get(&key)
                    .map(|e| (e.score, e.best_move.map(|m| m.column).unwrap_or(-1)))
            }
        }
    }

    pub fn lookup_canonical(&self, key: &PositionKey) -> Option<BookEntry> {
        if !self.covers_ply(ply_of_words(&key.mask)) {
            return None;
        }
        match &self.storage {
            Storage::Single { .. } => {
                if key.current.len() != 1 || key.mask.len() != 1 {
                    return None;
                }
                let score = self.probe(key.current[0], key.mask[0]);
                if score == MISS {
                    None
                } else {
                    Some(BookEntry {
                        score,
                        best_move: None,
                    })
                }
            }
            Storage::General { entries } => entries.get(key).copied(),
        }
    }

    pub fn lookup_entry(&self, position: &Position) -> Option<BookEntry> {
        let rules = position.rules();
        if let Some(book_rules) = &self.rules {
            if book_rules != rules {
                return None;
            }
        }
        if !self.may_contain_ply(position.moves_played() as i32) {
            return None;
        }
        match &self.storage {
            Storage::Single { .. } => {
                if rules.word_count() != 1 {
                    return None;
                }
                let cur = position.current_words()[0];
                let mask = position.mask_words()[0];
                let (cc, mm, flipped) = canonical_single_flipped(cur, mask, rules);
                let score = self.probe(cc, mm);
                if score == MISS {
                    return None;
                }
                let mut best_move = None;
                let col = self.best_col(cc, mm);
                if col >= 0 && col < rules.width {
                    let raw_col = if flipped { rules.width - 1 - col } else { col };
                    if rules.placement == Placement::Gravity {
                        let row = position.column_count(raw_col);
                        if row < rules.height {
                            best_move = Some(Move::cell(raw_col, row));
                        }
                    }
                }
                Some(BookEntry { score, best_move })
            }
            Storage::General { entries } => {
                let canonical = position.canonical_key();
                let entry = entries.get(&canonical).copied()?;
                let flipped = canonical != position.raw_key();
                let best_move = entry.best_move.map(|m| {
                    if flipped {
                        Move::cell(rules.width - 1 - m.column, m.row)
                    } else {
                        m
                    }
                });
                Some(BookEntry {
                    score: entry.score,
                    best_move,
                })
            }
        }
    }

    pub fn lookup_position(&self, position: &Position) -> i32 {
        self.lookup_entry(position).map(|e| e.score).unwrap_or(MISS)
    }

    pub fn entries(&self) -> Vec<(PositionKey, BookEntry)> {
        let mut out: Vec<(PositionKey, BookEntry)> = match &self.storage {
            Storage::Single {
                keys,
                scores,
                best_columns,
                ..
            } => keys
                .iter()
                .enumerate()
                .map(|(i, &(cur, mask))| {
                    let best_move = if best_columns[i] >= 0 {
                        let col = best_columns[i] as i32;
                        let row = match &self.rules {
                            Some(r) if r.placement == Placement::Gravity => {
                                count_in_column(mask, r, col)
                            }
                            _ => 0,
                        };
                        Some(Move::cell(col, row))
                    } else {
                        None
                    };
                    (
                        PositionKey {
                            current: Box::new([cur]),
                            mask: Box::new([mask]),
                        },
                        BookEntry {
                            score: scores[i],
                            best_move,
                        },
                    )
                })
                .collect(),
            Storage::General { entries } => entries.iter().map(|(k, e)| (k.clone(), *e)).collect(),
        };
        out.sort_by(|a, b| {
            a.0.current
                .cmp(&b.0.current)
                .then_with(|| a.0.mask.cmp(&b.0.mask))
        });
        out
    }

    pub fn canonical_keys(&self) -> Vec<PositionKey> {
        self.entries().into_iter().map(|(k, _)| k).collect()
    }

    pub fn load(path: &std::path::Path) -> Result<OpeningBook, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read book {}: {e}", path.display()))?;
        OpeningBook::parse(&text)
    }

    pub fn load_sections(path: &std::path::Path) -> Result<Vec<OpeningBook>, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read book {}: {e}", path.display()))?;
        OpeningBook::parse_sections(&text)
    }

    pub fn load_for_rules(
        path: &std::path::Path,
        rules: &Rules,
    ) -> Result<Option<OpeningBook>, String> {
        OpeningBook::load_for_rules_assuming(path, rules, true)
    }

    fn load_for_rules_assuming(
        path: &std::path::Path,
        rules: &Rules,
        assume_rules_if_absent: bool,
    ) -> Result<Option<OpeningBook>, String> {
        let sections = OpeningBook::load_sections(path)?;
        let matching: Vec<OpeningBook> = sections
            .into_iter()
            .filter(|b| {
                b.rules
                    .as_ref()
                    .map(|r| r == rules)
                    .unwrap_or(assume_rules_if_absent)
            })
            .map(|b| {
                if b.rules.is_none() {
                    b.with_rules(rules.clone())
                } else {
                    b
                }
            })
            .collect();
        if matching.is_empty() {
            Ok(None)
        } else {
            Ok(Some(OpeningBook::merged(matching)?))
        }
    }

    pub fn candidate_directories() -> Vec<PathBuf> {
        fn push_unique(dirs: &mut Vec<PathBuf>, keys: &mut Vec<PathBuf>, dir: PathBuf) {
            let key = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if !keys.contains(&key) {
                keys.push(key);
                dirs.push(dir);
            }
        }
        let mut dirs = Vec::new();
        let mut keys = Vec::new();
        if let Ok(dir) = std::env::var("CONNECTK_BOOK_DIR") {
            if !dir.trim().is_empty() {
                push_unique(&mut dirs, &mut keys, PathBuf::from(dir));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors() {
                push_unique(&mut dirs, &mut keys, ancestor.join("books"));
            }
        } else {
            push_unique(&mut dirs, &mut keys, PathBuf::from("books"));
        }
        if let Ok(exe) = std::env::current_exe() {
            for ancestor in exe.ancestors().skip(1) {
                push_unique(&mut dirs, &mut keys, ancestor.join("books"));
            }
        }
        #[cfg(target_os = "linux")]
        push_unique(
            &mut dirs,
            &mut keys,
            PathBuf::from("/usr/share/connectk/books"),
        );
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        push_unique(&mut dirs, &mut keys, manifest_dir.join("assets"));
        push_unique(&mut dirs, &mut keys, manifest_dir.join("../../books"));
        dirs
    }

    pub fn candidate_paths_for_rules(rules: &Rules) -> Vec<PathBuf> {
        let mut named: Vec<(usize, i32, u64, PathBuf)> = Vec::new();
        let mut others: Vec<PathBuf> = Vec::new();
        for (dir_index, dir) in OpeningBook::candidate_directories().into_iter().enumerate() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                paths.sort();
                for path in paths {
                    if path.extension().map(|x| x != "book").unwrap_or(true) {
                        continue;
                    }
                    if let Some((book_rules, depth)) = named_book_rules_depth(&path) {
                        if &book_rules == rules {
                            let len = path.metadata().map(|m| m.len()).unwrap_or(0);
                            named.push((dir_index, depth, len, path));
                        }
                    }
                }
            }
            for name in ["universal.book", "custom.book"] {
                let path = dir.join(name);
                if path.is_file() && !others.contains(&path) {
                    others.push(path);
                }
            }
        }
        named.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| a.3.cmp(&b.3))
        });
        let mut out: Vec<PathBuf> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for path in named.into_iter().map(|x| x.3).chain(others) {
            let key = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.contains(&key) {
                seen.push(key);
                out.push(path);
            }
        }
        out
    }

    fn load_candidate_for_rules(
        path: &std::path::Path,
        rules: &Rules,
    ) -> Result<Option<OpeningBook>, String> {
        let assume_rules_if_absent = named_book_rules_depth(path)
            .map(|(book_rules, _)| &book_rules == rules)
            .unwrap_or(false);
        OpeningBook::load_for_rules_assuming(path, rules, assume_rules_if_absent)
    }

    pub fn candidate_signature_for_rules(rules: &Rules) -> String {
        let mut signature = String::new();
        for path in OpeningBook::candidate_paths_for_rules(rules) {
            let (len, modified) = match path.metadata() {
                Ok(m) => {
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    (m.len(), modified)
                }
                Err(_) => (0, 0),
            };
            signature.push_str(&format!("{}:{len}:{modified};", path.display()));
        }
        signature
    }

    pub fn discover_all_for_rules(rules: &Rules) -> Option<(Vec<PathBuf>, OpeningBook)> {
        let mut loaded: Vec<(PathBuf, OpeningBook)> = Vec::new();
        for path in OpeningBook::candidate_paths_for_rules(rules) {
            if let Ok(Some(book)) = OpeningBook::load_candidate_for_rules(&path, rules) {
                if book.size() > 0 {
                    loaded.push((path, book));
                }
            }
        }
        if loaded.is_empty() {
            return None;
        }
        if loaded.len() == 1 {
            let (path, book) = loaded.into_iter().next().unwrap();
            return Some((vec![path], book));
        }
        let refs: Vec<&OpeningBook> = loaded.iter().map(|(_, book)| book).collect();
        match OpeningBook::merged_refs(&refs) {
            Ok(book) => {
                let paths = loaded.into_iter().map(|(path, _)| path).collect();
                Some((paths, book))
            }
            Err(_) => loaded
                .into_iter()
                .max_by_key(|(_, book)| (book.size(), book.depth()))
                .map(|(path, book)| (vec![path], book)),
        }
    }

    pub fn discover_for_rules(rules: &Rules) -> Option<(PathBuf, OpeningBook)> {
        let (paths, book) = OpeningBook::discover_all_for_rules(rules)?;
        Some((paths.into_iter().next()?, book))
    }

    pub fn parse(text: &str) -> Result<OpeningBook, String> {
        let mut sections = OpeningBook::parse_sections(text)?;
        match sections.len() {
            0 => Err("book contains no entries".into()),
            1 => Ok(sections.remove(0)),
            n => Err(format!(
                "universal book has {n} sections; select one with load_for_rules"
            )),
        }
    }

    pub fn parse_sections(text: &str) -> Result<Vec<OpeningBook>, String> {
        let mut sections: Vec<OpeningBook> = Vec::new();
        let mut builder = SectionBuilder::default();

        for (lineno, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                let rest = rest.trim();
                if rest.starts_with("section") {
                    if builder.started {
                        if let Some(book) = builder.finish()? {
                            sections.push(book);
                        }
                        builder = SectionBuilder::default();
                    }
                    builder.started = true;
                    if let Some(i) = rest.find("rules:") {
                        builder.rules = Some(parse_rules_line(&rest[i + 6..])?);
                    }
                    continue;
                }
                builder.started = true;
                if let Some(i) = rest.find("rules:") {
                    builder.rules = Some(parse_rules_line(&rest[i + 6..])?);
                }
                if let Some(i) = rest.find("depth=") {
                    let tail = &rest[i + 6..];
                    let num: String = tail
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '-')
                        .collect();
                    builder.depth = num.parse().unwrap_or(-1);
                }
                continue;
            }

            builder.started = true;
            if let Some(rest) = line.strip_prefix("cur=") {
                builder.entries.push(parse_general_entry(rest, lineno)?);
                builder.all_legacy = false;
            } else {
                builder.entries.push(parse_legacy_entry(line, lineno)?);
            }
        }
        if builder.started {
            if let Some(book) = builder.finish()? {
                sections.push(book);
            }
        }
        Ok(sections)
    }

    pub fn save(&self, path: &std::path::Path, rules: &Rules) -> Result<(), String> {
        let text = match &self.storage {
            Storage::Single {
                keys,
                scores,
                best_columns,
                ..
            } => {
                let mut s = String::with_capacity(keys.len() * 42 + 256);
                s.push_str(&format!("# connectk-book v1 depth={}\n", self.depth));
                s.push_str(&format!("# rules: {}\n", rules_line(rules)));
                for i in 0..keys.len() {
                    let (c, m) = keys[i];
                    s.push_str(&format!(
                        "{c:016x} {m:016x} {} {}\n",
                        scores[i], best_columns[i]
                    ));
                }
                s
            }
            Storage::General { .. } => {
                let mut s = String::from("# connectk-book v2 universal\n");
                s.push_str(&serialize_section(self, rules));
                s
            }
        };
        write_atomic(path, &text)
    }

    pub fn save_universal(path: &std::path::Path, books: &[OpeningBook]) -> Result<(), String> {
        let mut s = String::from("# connectk-book v2 universal\n");
        for book in books {
            let rules = book
                .rules
                .as_ref()
                .ok_or("universal book sections require rules metadata")?;
            s.push_str(&serialize_section(book, rules));
        }
        write_atomic(path, &s)
    }
}

enum PendingEntry {
    Legacy((u64, u64), i32, i16),
    General(PositionKey, BookEntry),
}

struct SectionBuilder {
    started: bool,
    depth: i32,
    rules: Option<Rules>,
    entries: Vec<PendingEntry>,
    all_legacy: bool,
}

impl SectionBuilder {
    fn finish(self) -> Result<Option<OpeningBook>, String> {
        if self.entries.is_empty() {
            return Ok(None);
        }
        let depth = self.depth;
        let rules = self.rules;

        if self.all_legacy {
            let mut rows: Vec<((u64, u64), i32, i16)> = self
                .entries
                .into_iter()
                .map(|e| match e {
                    PendingEntry::Legacy(key, score, best) => (key, score, best),
                    PendingEntry::General(..) => {
                        unreachable!("all_legacy is false for general entries")
                    }
                })
                .collect();
            rows.sort_by_key(|r| r.0);
            let mut keys: Vec<(u64, u64)> = Vec::with_capacity(rows.len());
            let mut scores: Vec<i32> = Vec::with_capacity(rows.len());
            let mut best_columns: Vec<i16> = Vec::with_capacity(rows.len());
            for (key, score, best) in rows {
                if keys.last() == Some(&key) {
                    let i = keys.len() - 1;
                    if scores[i] != score {
                        return Err(format!(
                            "conflicting book scores {} and {} for one position",
                            scores[i], score
                        ));
                    }
                    if best_columns[i] < 0 {
                        best_columns[i] = best;
                    }
                    continue;
                }
                keys.push(key);
                scores.push(score);
                best_columns.push(best);
            }
            let mut book = OpeningBook::new(keys, scores, best_columns, depth);
            book.rules = rules;
            return Ok(Some(book));
        }

        let entries: Vec<(PositionKey, BookEntry)> = self
            .entries
            .into_iter()
            .map(|e| match e {
                PendingEntry::General(key, entry) => (key, entry),
                PendingEntry::Legacy((cur, mask), score, best) => (
                    PositionKey {
                        current: Box::new([cur]),
                        mask: Box::new([mask]),
                    },
                    BookEntry {
                        score,
                        best_move: if best >= 0 {
                            Some(Move::cell(best as i32, 0))
                        } else {
                            None
                        },
                    },
                ),
            })
            .collect();
        Ok(Some(OpeningBook::from_entries(entries, depth, rules)?))
    }
}

impl Default for SectionBuilder {
    fn default() -> Self {
        SectionBuilder {
            started: false,
            depth: -1,
            rules: None,
            entries: Vec::new(),
            all_legacy: true,
        }
    }
}

fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("book.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot rename to {}: {e}", path.display()))?;
    Ok(())
}

fn serialize_section(book: &OpeningBook, rules: &Rules) -> String {
    let mut s = String::new();
    s.push_str(&format!("# section rules: {}\n", rules_line(rules)));
    s.push_str(&format!("# depth={}\n", book.depth));
    let compact = rules.word_count() == 1 && rules.placement == Placement::Gravity;
    for (key, entry) in book.entries() {
        if compact {
            let col = entry.best_move.map(|m| m.column).unwrap_or(-1);
            s.push_str(&format!(
                "{:016x} {:016x} {} {}\n",
                key.current[0], key.mask[0], entry.score, col
            ));
        } else {
            let cur = words_hex(&key.current);
            let mask = words_hex(&key.mask);
            let best = entry
                .best_move
                .map(|m| format!("{},{}", m.column, m.row))
                .unwrap_or_else(|| "-".to_string());
            s.push_str(&format!(
                "cur={cur} mask={mask} score={} best={best}\n",
                entry.score
            ));
        }
    }
    s
}

fn words_hex(words: &[u64]) -> String {
    words
        .iter()
        .map(|w| format!("{w:016x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_legacy_entry(line: &str, lineno: usize) -> Result<PendingEntry, String> {
    let mut it = line.split_whitespace();
    let (Some(c), Some(m), Some(s)) = (it.next(), it.next(), it.next()) else {
        return Err(format!("bad book entry on line {}", lineno + 1));
    };
    let cur = parse_hex(c, lineno)?;
    let mask = parse_hex(m, lineno)?;
    let score: i32 = s
        .parse()
        .map_err(|_| format!("bad score on book line {}", lineno + 1))?;
    let bc: i32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(-1);
    Ok(PendingEntry::Legacy((cur, mask), score, bc as i16))
}

fn parse_general_entry(rest: &str, lineno: usize) -> Result<PendingEntry, String> {
    let mut current: Option<Vec<u64>> = None;
    let mut mask: Option<Vec<u64>> = None;
    let mut score: Option<i32> = None;
    let mut best: Option<Move> = None;

    let first = format!("cur={rest}");
    for token in first.split_whitespace() {
        let Some((k, v)) = token.split_once('=') else {
            return Err(format!("bad book entry on line {}", lineno + 1));
        };
        match k {
            "cur" => current = Some(parse_words(v, lineno)?),
            "mask" => mask = Some(parse_words(v, lineno)?),
            "score" => {
                score = Some(
                    v.parse()
                        .map_err(|_| format!("bad score on book line {}", lineno + 1))?,
                );
            }
            "best" => {
                if v != "-" {
                    let Some((c, r)) = v.split_once(',') else {
                        return Err(format!("bad best move on book line {}", lineno + 1));
                    };
                    let col: i32 = c
                        .parse()
                        .map_err(|_| format!("bad best move on book line {}", lineno + 1))?;
                    let row: i32 = r
                        .parse()
                        .map_err(|_| format!("bad best move on book line {}", lineno + 1))?;
                    best = Some(Move::cell(col, row));
                }
            }
            _ => return Err(format!("unknown book field '{k}' on line {}", lineno + 1)),
        }
    }
    let (Some(current), Some(mask), Some(score)) = (current, mask, score) else {
        return Err(format!("incomplete book entry on line {}", lineno + 1));
    };
    if current.len() != mask.len() || current.is_empty() {
        return Err(format!(
            "book key word-count mismatch on line {}",
            lineno + 1
        ));
    }
    Ok(PendingEntry::General(
        PositionKey {
            current: current.into_boxed_slice(),
            mask: mask.into_boxed_slice(),
        },
        BookEntry {
            score,
            best_move: best,
        },
    ))
}

fn parse_words(s: &str, lineno: usize) -> Result<Vec<u64>, String> {
    s.split(',').map(|w| parse_hex(w, lineno)).collect()
}

fn parse_hex(s: &str, lineno: usize) -> Result<u64, String> {
    u64::from_str_radix(s, 16).map_err(|_| format!("bad hex key on book line {}", lineno + 1))
}

fn count_in_column(mask: u64, rules: &Rules, column: i32) -> i32 {
    let mut n = 0;
    for r in 0..rules.height {
        let idx = rules.index(column, r);
        if (0..64).contains(&idx) && (mask >> idx) & 1 != 0 {
            n += 1;
        }
    }
    n
}

fn named_book_rules_depth(path: &Path) -> Option<(Rules, i32)> {
    fn digits(text: &str) -> Option<(&str, &str)> {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        if end == 0 {
            None
        } else {
            Some((&text[..end], &text[end..]))
        }
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    let stem = name.strip_suffix(".book")?;
    let (diagonals, rest) = if let Some(rest) = stem.strip_prefix("std") {
        (true, rest)
    } else {
        (false, stem.strip_prefix("nodiag")?)
    };
    let (width, rest) = digits(rest)?;
    let rest = rest.strip_prefix('x')?;
    let (height, rest) = digits(rest)?;
    let (connect, rest) = if let Some(rest) = rest.strip_prefix("-k") {
        let (connect, rest) = digits(rest)?;
        (connect.parse().ok()?, rest)
    } else {
        (4, rest)
    };
    let rest = rest.strip_prefix("-d")?;
    let (depth, _) = digits(rest)?;
    let rules = Rules::new(
        width.parse().ok()?,
        height.parse().ok()?,
        connect,
        Placement::Gravity,
        if diagonals {
            dirs::ALL
        } else {
            dirs::ORTHOGONAL
        },
        false,
    )
    .ok()?;
    Some((rules, depth.parse().ok()?))
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

pub fn parse_rules_line(text: &str) -> Result<Rules, String> {
    let mut width: Option<i32> = None;
    let mut height: Option<i32> = None;
    let mut connect: Option<i32> = None;
    let mut placement = Placement::Gravity;
    let mut directions: u8 = 0;
    let mut misere = false;

    for token in text.split_whitespace() {
        let Some((k, v)) = token.split_once('=') else {
            continue;
        };
        match k {
            "width" => width = v.parse().ok(),
            "height" => height = v.parse().ok(),
            "K" | "k" | "connect" => connect = v.parse().ok(),
            "placement" => {
                placement = if v.eq_ignore_ascii_case("FREE") {
                    Placement::Free
                } else {
                    Placement::Gravity
                }
            }
            "directions" => {
                for name in v.split(',') {
                    directions |= match name.trim() {
                        "V" => dirs::VERTICAL,
                        "H" => dirs::HORIZONTAL,
                        "DU" => dirs::DIAGONAL_UP,
                        "DD" => dirs::DIAGONAL_DOWN,
                        _ => 0,
                    };
                }
            }
            "misere" => misere = v.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    let (Some(width), Some(height), Some(connect)) = (width, height, connect) else {
        return Err(format!("incomplete book rules line: '{text}'"));
    };
    if directions == 0 {
        directions = dirs::ALL;
    }
    Rules::new(width, height, connect, placement, directions, misere)
}

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

#[inline]
pub fn mirror_word(bits: u64, rules: &Rules) -> u64 {
    let stride = rules.stride() as u32;
    let col_mask: u64 = if rules.height >= 64 {
        u64::MAX
    } else {
        (1u64 << rules.height) - 1
    };
    let mut out = 0u64;
    for c in 0..rules.width as u32 {
        let piece = (bits >> (c * stride)) & col_mask;
        out |= piece << ((rules.width as u32 - 1 - c) * stride);
    }
    out
}
