use crate::bits;
use crate::model::{Move, Outcome, Placement, Player, Rules};
use core::hash::{Hash, Hasher};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PositionKey {
    pub current: Box<[u64]>,
    pub mask: Box<[u64]>,
}

impl Hash for PositionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for w in self.current.iter() {
            state.write_u64(*w);
        }
        for w in self.mask.iter() {
            state.write_u64(*w);
        }
    }
}

#[derive(Clone)]
pub struct Position {
    rules: Rules,
    current: Vec<u64>,
    mask: Vec<u64>,
    column_counts: Vec<u16>,
    history: Vec<Move>,
    moves: u32,
}

impl Position {
    pub fn initial(rules: &Rules) -> Position {
        Position {
            rules: rules.clone(),
            current: vec![0; rules.word_count()],
            mask: vec![0; rules.word_count()],
            column_counts: vec![0; rules.width as usize],
            history: Vec::new(),
            moves: 0,
        }
    }

    pub fn from_move_sequence(rules: &Rules, moves: &[Move]) -> Result<Position, String> {
        let mut p = Position::initial(rules);
        for (i, &m) in moves.iter().enumerate() {
            p.play(m)?;
            if i + 1 < moves.len() && p.is_terminal() {
                return Err(format!(
                    "moves continue after the game has ended (at move {})",
                    i + 1
                ));
            }
        }
        Ok(p)
    }

    pub fn from_words(rules: &Rules, current: &[u64], mask: &[u64]) -> Result<Position, String> {
        let n = rules.word_count();
        if current.len() != n || mask.len() != n {
            return Err(format!(
                "expected {n} bitboard words, got {} current and {} mask",
                current.len(),
                mask.len()
            ));
        }
        let board = rules.board_mask();
        for i in 0..n {
            if current[i] & !mask[i] != 0 {
                return Err("current-player stones are not a subset of all stones".into());
            }
            if mask[i] & !board[i] != 0 {
                return Err("stones outside the board".into());
            }
        }
        let moves: u32 = mask.iter().map(|w| w.count_ones()).sum();
        let current_stones: u32 = current.iter().map(|w| w.count_ones()).sum();
        if current_stones != moves / 2 {
            return Err(format!(
                "stone counts do not match the side to move: {current_stones} current in {moves} total"
            ));
        }
        let mut column_counts = vec![0u16; rules.width as usize];
        for c in 0..rules.width {
            let mut count = 0u16;
            for r in 0..rules.height {
                if bits::test(mask, rules.index(c, r)) {
                    count += 1;
                }
            }
            if rules.placement == Placement::Gravity {
                for r in 0..count as i32 {
                    if !bits::test(mask, rules.index(c, r)) {
                        return Err("gravity stones are not stacked from the bottom".into());
                    }
                }
                for r in count as i32..rules.height {
                    if bits::test(mask, rules.index(c, r)) {
                        return Err("gravity stones are not stacked from the bottom".into());
                    }
                }
            }
            if c >= 0 {
                column_counts[c as usize] = count;
            }
        }
        Ok(Position {
            rules: rules.clone(),
            current: current.to_vec(),
            mask: mask.to_vec(),
            column_counts,
            history: Vec::new(),
            moves,
        })
    }

    pub fn from_board<S: AsRef<str>>(
        rules: &Rules,
        rows_top_first: &[S],
    ) -> Result<Position, String> {
        let w = rules.width;
        let h = rules.height;
        if rows_top_first.len() != h as usize {
            return Err(format!("expected {h} rows, got {}", rows_top_first.len()));
        }
        let mut one = vec![0u64; rules.word_count()];
        let mut two = vec![0u64; rules.word_count()];
        let (mut n1, mut n2) = (0i32, 0i32);
        for (i, row) in rows_top_first.iter().enumerate() {
            let row = row.as_ref();
            if row.chars().count() != w as usize {
                return Err(format!("row {i} must have {w} cells: '{row}'"));
            }
            let r = h - 1 - i as i32;
            for (c, ch) in row.chars().enumerate() {
                let idx = rules.index(c as i32, r);
                match ch {
                    'X' => {
                        bits::set(&mut one, idx);
                        n1 += 1;
                    }
                    'O' => {
                        bits::set(&mut two, idx);
                        n2 += 1;
                    }
                    '.' => {}
                    _ => return Err(format!("illegal board character '{ch}' (use X, O, .)")),
                }
            }
        }
        if !(n1 == n2 || n1 == n2 + 1) {
            return Err(format!(
                "impossible stone counts: X={n1}, O={n2} (X moves first, so X count must \
                 equal O count or exceed it by one)"
            ));
        }
        let mut p = Position::initial(rules);
        bits::or_into(&one, &two, &mut p.mask);
        let to_move = if n1 == n2 { Player::One } else { Player::Two };
        for c in 0..w {
            let mut cnt = 0u16;
            for r in 0..h {
                if bits::test(&p.mask, rules.index(c, r)) {
                    cnt += 1;
                }
            }
            if rules.placement == Placement::Gravity {
                for r in 0..h {
                    let filled = bits::test(&p.mask, rules.index(c, r));
                    if filled != (r < cnt as i32) {
                        return Err(format!(
                            "floating stone in column {} (gravity placement)",
                            c + 1
                        ));
                    }
                }
            }
            p.column_counts[c as usize] = cnt;
        }
        p.moves = (n1 + n2) as u32;
        p.current = match to_move {
            Player::One => one,
            Player::Two => two,
        };
        if rules.connect != 0 && p.has_line(Player::One) && p.has_line(Player::Two) {
            return Err("both players have a completed line".into());
        }
        if rules.connect != 0 && p.has_line(to_move) {
            return Err(
                "the side to move already has a completed line (game would have ended)".into(),
            );
        }
        Ok(p)
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }
    pub fn moves_played(&self) -> u32 {
        self.moves
    }
    pub fn side_to_move(&self) -> Player {
        if self.moves.is_multiple_of(2) {
            Player::One
        } else {
            Player::Two
        }
    }
    pub fn move_history(&self) -> &[Move] {
        &self.history
    }
    pub fn last_move(&self) -> Option<Move> {
        self.history.last().copied()
    }
    pub fn column_count(&self, column: i32) -> i32 {
        self.column_counts[column as usize] as i32
    }
    pub fn is_empty(&self, column: i32, row: i32) -> bool {
        !bits::test(&self.mask, self.rules.index(column, row))
    }
    pub fn owner_at(&self, column: i32, row: i32) -> Option<Player> {
        let idx = self.rules.index(column, row);
        if !bits::test(&self.mask, idx) {
            None
        } else if bits::test(&self.current, idx) {
            Some(self.side_to_move())
        } else {
            Some(self.side_to_move().other())
        }
    }

    pub fn current_words(&self) -> &[u64] {
        &self.current
    }
    pub fn mask_words(&self) -> &[u64] {
        &self.mask
    }

    pub fn stone_bits(&self, player: Player) -> Vec<u64> {
        let mut out = vec![0u64; self.rules.word_count()];
        self.write_stone_bits(player, &mut out);
        out
    }
    pub fn write_stone_bits(&self, player: Player, out: &mut [u64]) {
        if player == self.side_to_move() {
            out.copy_from_slice(&self.current);
        } else {
            bits::xor_into(&self.mask, &self.current, out);
        }
    }

    fn has_line_words(&self, stones: &[u64]) -> bool {
        let k = self.rules.connect;
        if k == 0 {
            return true;
        }
        if k > self.rules.max_line() {
            return false;
        }
        if stones.len() == 1 {
            let b = stones[0];

            for &s in self.rules.shifts() {
                let mut x = b;
                for i in 1..k {
                    x &= b >> (s * i as u32);
                }
                if x != 0 {
                    return true;
                }
            }
            return false;
        }
        let n = stones.len();
        let mut acc = vec![0u64; n];
        let mut tmp = vec![0u64; n];
        for &s in self.rules.shifts() {
            acc.copy_from_slice(stones);
            for i in 1..k {
                bits::shift_right(stones, s * i as u32, &mut tmp);
                bits::and_assign(&mut acc, &tmp);
                if !bits::any(&acc) {
                    break;
                }
            }
            if bits::any(&acc) {
                return true;
            }
        }
        false
    }

    pub fn has_line(&self, player: Player) -> bool {
        if self.rules.word_count() == 1 {
            let stones = if player == self.side_to_move() {
                self.current[0]
            } else {
                self.mask[0] ^ self.current[0]
            };
            self.has_line_words(&[stones])
        } else {
            let stones = self.stone_bits(player);
            self.has_line_words(&stones)
        }
    }

    pub fn line_maker(&self) -> Option<Player> {
        if self.rules.connect == 0 {
            return Some(Player::One);
        }
        if self.rules.word_count() > 1 {
            if let Some(&last) = self.history.last() {
                let prev = self.side_to_move().other();
                return if self.has_line_through(last, prev) {
                    Some(prev)
                } else {
                    None
                };
            }
        }
        let prev = self.side_to_move().other();
        if self.has_line(prev) {
            Some(prev)
        } else if self.has_line(self.side_to_move()) {
            Some(self.side_to_move())
        } else {
            None
        }
    }

    #[inline]
    fn has_line_through(&self, cell: Move, player: Player) -> bool {
        let side = self.side_to_move();
        let is_stone = |c: i32, r: i32| {
            if c < 0 || c >= self.rules.width || r < 0 || r >= self.rules.height {
                return false;
            }
            let idx = self.rules.index(c, r);
            if player == side {
                bits::test(&self.current, idx)
            } else {
                bits::test(&self.mask, idx) && !bits::test(&self.current, idx)
            }
        };
        if !is_stone(cell.column, cell.row) {
            return false;
        }
        for &(dc, dr) in self.rules.dir_steps() {
            let mut count = 1;
            let mut step = 1;
            while step < self.rules.connect
                && is_stone(cell.column + dc * step, cell.row + dr * step)
            {
                count += 1;
                step += 1;
            }
            step = 1;
            while step < self.rules.connect
                && is_stone(cell.column - dc * step, cell.row - dr * step)
            {
                count += 1;
                step += 1;
            }
            if count >= self.rules.connect {
                return true;
            }
        }
        false
    }

    pub fn winner(&self) -> Option<Player> {
        let maker = self.line_maker()?;
        Some(if self.rules.misere {
            maker.other()
        } else {
            maker
        })
    }

    pub fn is_full(&self) -> bool {
        self.moves as i32 == self.rules.cells()
    }
    pub fn is_terminal(&self) -> bool {
        self.is_full() || self.line_maker().is_some()
    }
    pub fn is_draw(&self) -> bool {
        self.is_full() && self.line_maker().is_none()
    }

    pub fn outcome_for_side_to_move(&self) -> Option<Outcome> {
        if let Some(w) = self.winner() {
            Some(if w == self.side_to_move() {
                Outcome::Win
            } else {
                Outcome::Loss
            })
        } else if self.is_full() {
            Some(Outcome::Draw)
        } else {
            None
        }
    }

    pub fn completed_line_cells(&self) -> Vec<Move> {
        let Some(maker) = self.line_maker() else {
            return Vec::new();
        };
        let k = self.rules.connect;
        let mut stones = vec![0u64; self.rules.word_count()];
        self.write_stone_bits(maker, &mut stones);
        for &(dc, dr) in self.rules.dir_steps() {
            for c in 0..self.rules.width {
                for r in 0..self.rules.height {
                    let (ec, er) = (c + dc * (k - 1), r + dr * (k - 1));
                    if ec < 0 || ec >= self.rules.width || er < 0 || er >= self.rules.height {
                        continue;
                    }
                    let mut cells = Vec::with_capacity(k as usize);
                    let mut all = true;
                    for i in 0..k {
                        let (cc, rr) = (c + dc * i, r + dr * i);
                        if !bits::test(&stones, self.rules.index(cc, rr)) {
                            all = false;
                            break;
                        }
                        cells.push(Move::cell(cc, rr));
                    }
                    if all {
                        return cells;
                    }
                }
            }
        }
        Vec::new()
    }

    pub fn winning_spots(&self, player: Player) -> Vec<u64> {
        let mut out = vec![0u64; self.rules.word_count()];
        self.write_winning_spots(player, &mut out);
        out
    }

    pub fn write_winning_spots(&self, player: Player, spots: &mut [u64]) {
        let k = self.rules.connect;
        let n = self.rules.word_count();
        for w in spots.iter_mut() {
            *w = 0;
        }
        if k <= 0 || k > self.rules.max_line() {
            return;
        }
        if n == 1 {
            let stones = if player == self.side_to_move() {
                self.current[0]
            } else {
                self.mask[0] ^ self.current[0]
            };
            let board = self.rules.board_mask()[0];
            let mut result = 0u64;
            for &shift in self.rules.shifts() {
                for hole in 0..k {
                    let mut cand = board;
                    for i in 0..k {
                        if i == hole {
                            continue;
                        }
                        cand &= bits::signed_shift(stones, -(i - hole) * shift as i32);
                        if cand == 0 {
                            break;
                        }
                    }
                    result |= cand;
                }
            }
            spots[0] = result & board & !self.mask[0];
            return;
        }
        if n <= 4 {
            let mut stones = [0u64; 4];
            let mut cand = [0u64; 4];
            let mut tmp = [0u64; 4];
            self.write_stone_bits(player, &mut stones[..n]);
            self.write_winning_spots_words(&stones[..n], spots, &mut cand[..n], &mut tmp[..n]);
            return;
        }
        let mut stones = vec![0u64; n];
        self.write_stone_bits(player, &mut stones);
        let mut cand = vec![0u64; n];
        let mut tmp = vec![0u64; n];
        self.write_winning_spots_words(&stones, spots, &mut cand, &mut tmp);
    }

    fn write_winning_spots_words(
        &self,
        stones: &[u64],
        spots: &mut [u64],
        cand: &mut [u64],
        tmp: &mut [u64],
    ) {
        let k = self.rules.connect;
        let board = self.rules.board_mask();
        for &shift in self.rules.shifts() {
            for hole in 0..k {
                cand.copy_from_slice(board);
                for i in 0..k {
                    if i == hole {
                        continue;
                    }
                    bits::signed_shift_words(stones, -(i - hole) * shift as i32, tmp);
                    bits::and_assign(cand, tmp);
                    if !bits::any(cand) {
                        break;
                    }
                }
                bits::or_assign(spots, cand);
            }
        }
        for i in 0..spots.len() {
            spots[i] &= board[i] & !self.mask[i];
        }
    }

    pub fn count_winning_spots(&self, player: Player) -> u32 {
        bits::bit_count(&self.winning_spots(player))
    }

    pub fn line_completing_moves(&self, player: Player) -> Vec<Move> {
        if self.is_terminal() {
            return Vec::new();
        }
        let spots = self.winning_spots(player);
        self.legal_moves()
            .into_iter()
            .filter(|m| bits::test(&spots, self.rules.index(m.column, m.row)))
            .collect()
    }

    pub fn completes_line(&self, m: Move) -> bool {
        match self.resolve(m) {
            Ok(cell) => {
                let spots = self.winning_spots(self.side_to_move());
                bits::test(&spots, self.rules.index(cell.column, cell.row))
            }
            Err(_) => false,
        }
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        let mut out = Vec::new();
        if self.is_terminal() {
            return out;
        }
        match self.rules.placement {
            Placement::Gravity => {
                for c in 0..self.rules.width {
                    let cnt = self.column_count(c);
                    if cnt < self.rules.height {
                        out.push(Move::cell(c, cnt));
                    }
                }
            }
            Placement::Free => {
                for (word, &board) in self.rules.board_mask().iter().enumerate() {
                    let mut empty = board & !self.mask[word];
                    while empty != 0 {
                        let idx = (word * 64) as i32 + empty.trailing_zeros() as i32;
                        out.push(Move::cell(
                            self.rules.column_of(idx),
                            self.rules.row_of(idx),
                        ));
                        empty &= empty - 1;
                    }
                }
            }
        }
        out
    }

    pub fn resolve(&self, m: Move) -> Result<Move, String> {
        let c = m.column;
        if c < 0 || c >= self.rules.width {
            return Err(format!(
                "column {} out of range 1..{}",
                c + 1,
                self.rules.width
            ));
        }
        match self.rules.placement {
            Placement::Gravity => {
                let landing = self.column_count(c);
                if landing >= self.rules.height {
                    return Err(format!("column {} is full", c + 1));
                }
                if m.is_resolved() && m.row != landing {
                    return Err(format!(
                        "under gravity a stone in column {} lands on row {}, not {}",
                        c + 1,
                        landing + 1,
                        m.row + 1
                    ));
                }
                Ok(Move::cell(c, landing))
            }
            Placement::Free => {
                if !m.is_resolved() {
                    return Err(format!("free placement needs a row: use {}:<row>", c + 1));
                }
                if m.row < 0 || m.row >= self.rules.height {
                    return Err(format!(
                        "row {} out of range 1..{}",
                        m.row + 1,
                        self.rules.height
                    ));
                }
                if !self.is_empty(c, m.row) {
                    return Err(format!("cell {}:{} is occupied", c + 1, m.row + 1));
                }
                Ok(m)
            }
        }
    }

    pub fn play(&mut self, m: Move) -> Result<Move, String> {
        if self.is_terminal() {
            return Err("the game is already over".into());
        }
        let cell = self.resolve(m)?;
        self.apply_resolved(cell);
        Ok(cell)
    }

    pub fn play_column(&mut self, column: i32) -> Result<Move, String> {
        self.play(Move::column(column))
    }

    pub fn undo(&mut self) -> Result<Move, String> {
        let Some(&last) = self.history.last() else {
            return Err("nothing to undo".into());
        };
        self.unapply_resolved(last);
        Ok(last)
    }

    #[inline]
    pub(crate) fn apply_resolved(&mut self, cell: Move) {
        for i in 0..self.current.len() {
            self.current[i] ^= self.mask[i];
        }
        bits::set(&mut self.mask, self.rules.index(cell.column, cell.row));
        self.column_counts[cell.column as usize] += 1;
        self.moves += 1;
        self.history.push(cell);
    }

    #[inline]
    pub(crate) fn unapply_resolved(&mut self, cell: Move) {
        self.history.pop();
        self.moves -= 1;
        self.column_counts[cell.column as usize] -= 1;
        bits::clear(&mut self.mask, self.rules.index(cell.column, cell.row));
        for i in 0..self.current.len() {
            self.current[i] ^= self.mask[i];
        }
    }

    pub fn raw_key(&self) -> PositionKey {
        PositionKey {
            current: self.current.clone().into_boxed_slice(),
            mask: self.mask.clone().into_boxed_slice(),
        }
    }

    pub fn canonical_key(&self) -> PositionKey {
        if !self.rules.mirror_symmetric() {
            return self.raw_key();
        }
        let mm = mirror_words(&self.mask, &self.rules);
        let cmp = bits::compare_unsigned(&mm, &self.mask);
        if cmp == core::cmp::Ordering::Greater {
            return self.raw_key();
        }
        let mc = mirror_words(&self.current, &self.rules);
        if cmp == core::cmp::Ordering::Equal
            && bits::compare_unsigned(&mc, &self.current) != core::cmp::Ordering::Less
        {
            return self.raw_key();
        }
        PositionKey {
            current: mc.into_boxed_slice(),
            mask: mm.into_boxed_slice(),
        }
    }
}

fn mirror_words(words: &[u64], rules: &Rules) -> Vec<u64> {
    let mut out = vec![0u64; words.len()];
    for c in 0..rules.width {
        for r in 0..rules.height {
            if bits::test(words, rules.index(c, r)) {
                bits::set(&mut out, rules.index(rules.width - 1 - c, r));
            }
        }
    }
    out
}

impl core::fmt::Display for Position {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = String::new();
        let field = self.rules.width.to_string().len();
        for r in (0..self.rules.height).rev() {
            for c in 0..self.rules.width {
                if c > 0 {
                    s.push(' ');
                }
                let symbol = match self.owner_at(c, r) {
                    Some(p) => p.symbol(),
                    None => '.',
                };
                s.push_str(&format!("{symbol:>field$}"));
            }
            s.push('\n');
        }
        for c in 0..self.rules.width {
            if c > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{:>field$}", c + 1));
        }
        write!(f, "{s}")
    }
}
